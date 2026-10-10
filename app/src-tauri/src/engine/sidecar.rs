//! A helper process Purl owns: the bundled `llama-server`. It listens on
//! 127.0.0.1 only, on a free port picked at spawn, and dies with the app:
//! `Drop` kills it, a Job Object kills it on Windows if Purl itself dies,
//! and on macOS a PID file lets the next launch reap a leftover.
//!
//! Another local program could grab the port between our pick and the
//! server's bind and pose as it, reading transcripts and returning text we
//! would paste. So a started server must prove itself: the listening socket
//! belongs to our child, and it refuses a wrong API key, accepts ours and
//! names our model.

use std::ffi::OsString;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a fresh server may take to load its model and answer `/health`.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_POLL: Duration = Duration::from_millis(100);
/// Name of the server binary, also how a stale PID is recognised as ours.
const SERVER_NAME: &str = "llama-server";

/// The bundled `llama-server` under the app's resource dir.
pub fn llama_server_path(resource_dir: &Path) -> PathBuf {
    let exe = if cfg!(windows) {
        "llama-server.exe"
    } else {
        SERVER_NAME
    };
    resource_dir.join("llama").join(exe)
}

/// Arguments for serving `model` on `port`. Thinking is switched off
/// (`--reasoning-budget 0`); one slot gets the whole context; no web UI and
/// no network beyond the loopback socket.
pub fn llama_args(model: &Path, port: u16) -> Vec<OsString> {
    // The Windows build is CPU-only, so there is nothing to offload to.
    let gpu_layers = if cfg!(windows) { "0" } else { "99" };
    let mut args: Vec<OsString> = [
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "-c",
        "4096",
        "-np",
        "1",
        "--jinja",
        "--reasoning-budget",
        "0",
        "-ngl",
        gpu_layers,
        "--no-webui",
        "--offline",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push("-m".into());
    args.push(model.into());
    args
}

/// A port nothing is listening on right now. The OS picks it; there is a
/// small window before the server binds it, which the caller covers by
/// retrying once on a fresh port.
pub fn free_port() -> io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// A random API key for one server run, so other local processes (or a web
/// page poking at localhost) can't use it: 128 bits from the OS CSPRNG.
/// Never logged, never put in an error.
pub fn api_key() -> String {
    crate::secure_random::hex(16)
}

pub struct Sidecar {
    child: Child,
    port: u16,
    pid_file: Option<PathBuf>,
    #[cfg(windows)]
    _job: job::Job,
}

impl Sidecar {
    /// Spawn `llama-server` for `model` on a free loopback port, wait until
    /// it is healthy, and check it is really ours (see `verify_identity`).
    /// The API key is passed in the environment, not on the command line, so
    /// it doesn't show in a process list.
    pub fn spawn_llama(
        bin: &Path,
        model: &Path,
        api_key: &str,
        pid_file: Option<&Path>,
    ) -> io::Result<Sidecar> {
        let port = free_port()?;
        let mut cmd = Command::new(bin);
        cmd.args(llama_args(model, port))
            .env("LLAMA_API_KEY", api_key);
        let sidecar = Self::start(cmd, port, HEALTH_TIMEOUT, pid_file)?;
        verify_identity(port, api_key, model)?;
        Ok(sidecar)
    }

    /// Start `cmd` (which must serve `GET /health` on `port`), wait up to
    /// `health_timeout` for a 200, then check the child itself is the one
    /// listening. The child is killed on any failure.
    pub fn start(
        mut cmd: Command,
        port: u16,
        health_timeout: Duration,
        pid_file: Option<&Path>,
    ) -> io::Result<Sidecar> {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(windows)]
        let job = job::Job::kill_on_close()?;
        let child = cmd.spawn()?;
        // Built before anything else can fail, so `Drop` cleans up from here.
        let mut sidecar = Sidecar {
            child,
            port,
            pid_file: pid_file.map(Path::to_path_buf),
            #[cfg(windows)]
            _job: job,
        };
        #[cfg(windows)]
        sidecar._job.assign(&sidecar.child)?;
        if let Some(path) = &sidecar.pid_file {
            std::fs::write(path, sidecar.child.id().to_string())?;
        }
        sidecar.wait_healthy(health_timeout)?;
        sidecar.verify_owner()?;
        Ok(sidecar)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// False once the process has exited.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Every socket listening on our port belongs to our child, which is
    /// still running. Fails closed when the owner can't be read.
    fn verify_owner(&mut self) -> io::Result<()> {
        let pids = listener_pids(self.port)?;
        if !self.is_alive() || pids.is_empty() || pids.iter().any(|&p| p != self.pid()) {
            return Err(io::Error::other(format!(
                "port {} is held by another program, not {SERVER_NAME}",
                self.port
            )));
        }
        Ok(())
    }

    fn wait_healthy(&mut self, timeout: Duration) -> io::Result<()> {
        let client = loopback_client(Duration::from_secs(2))?;
        let url = format!("http://127.0.0.1:{}/health", self.port);
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!(
                    "{SERVER_NAME} exited during start-up ({status})"
                )));
            }
            if client
                .get(&url)
                .send()
                .is_ok_and(|r| r.status().is_success())
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{SERVER_NAME} wasn't ready after {}s", timeout.as_secs()),
                ));
            }
            std::thread::sleep(HEALTH_POLL);
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(path) = &self.pid_file {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Check the server on `port` is llama-server holding `api_key` and serving
/// `model`: a random wrong key gets 401, ours gets 200 from the key-guarded
/// `/props`, which names the model. Errors never contain either key.
pub fn verify_identity(port: u16, api_key: &str, model: &Path) -> io::Result<()> {
    let client = loopback_client(Duration::from_secs(5))?;
    let url = format!("http://127.0.0.1:{port}/props");
    let not_ours = |why: &str| {
        io::Error::other(format!(
            "the server on port {port} isn't Purl's {SERVER_NAME}: {why}"
        ))
    };
    let props = |key: &str| {
        client
            .get(&url)
            .bearer_auth(key)
            .send()
            .map_err(|e| not_ours(&format!("no answer ({})", e.without_url())))
    };
    let wrong = props(&crate::secure_random::hex(16))?;
    if wrong.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Err(not_ours(&format!(
            "it accepted a wrong key ({})",
            wrong.status()
        )));
    }
    let right = props(api_key)?;
    if !right.status().is_success() {
        return Err(not_ours(&format!(
            "it refused our key ({})",
            right.status()
        )));
    }
    let body: serde_json::Value = right
        .json()
        .map_err(|_| not_ours("its /props isn't JSON"))?;
    if body["model_path"].as_str() != Some(&*model.to_string_lossy()) {
        return Err(not_ours("it serves a different model"));
    }
    Ok(())
}

/// PIDs of the processes listening on 127.0.0.1:`port`, via `lsof`.
#[cfg(unix)]
fn listener_pids(port: u16) -> io::Result<Vec<u32>> {
    // Absolute path on macOS so a PATH entry can't stand in for it.
    let lsof = if Path::new("/usr/sbin/lsof").exists() {
        "/usr/sbin/lsof"
    } else {
        "lsof"
    };
    let out = Command::new(lsof)
        .args([
            "-nP",
            &format!("-iTCP@127.0.0.1:{port}"),
            "-sTCP:LISTEN",
            "-t",
        ])
        .stderr(Stdio::null())
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect())
}

/// PIDs of the processes listening on 127.0.0.1:`port`, from the IPv4 TCP
/// table.
#[cfg(windows)]
fn listener_pids(port: u16) -> io::Result<Vec<u32>> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows::Win32::Networking::WinSock::AF_INET;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const LOOPBACK: u32 = u32::from_ne_bytes([127, 0, 0, 1]);
    // The table can grow between the size query and the read; retry.
    let mut size = 0u32;
    for _ in 0..4 {
        // u32 words keep the buffer aligned for the table's u32 fields.
        let mut buf = vec![0u32; (size as usize).div_ceil(4).max(1)];
        let rc = unsafe {
            GetExtendedTcpTable(
                Some(buf.as_mut_ptr().cast()),
                &mut size,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if rc == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if rc != 0 {
            return Err(io::Error::other(format!(
                "couldn't read the TCP table ({rc})"
            )));
        }
        let table = unsafe { &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
        let rows = unsafe {
            std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize)
        };
        return Ok(rows
            .iter()
            // The port is in network byte order in the low 16 bits.
            .filter(|r| u16::from_be(r.dwLocalPort as u16) == port)
            .filter(|r| r.dwLocalAddr == LOOPBACK || r.dwLocalAddr == 0)
            .map(|r| r.dwOwningPid)
            .collect());
    }
    Err(io::Error::other("the TCP table kept changing"))
}

/// An HTTP client for the loopback server: never through a proxy, and every
/// request bounded by `timeout`.
pub fn loopback_client(timeout: Duration) -> io::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .timeout(timeout)
        .build()
        .map_err(io::Error::other)
}

/// Kill a `llama-server` left running by a previous Purl that crashed, then
/// remove its PID file. A PID that now belongs to some other program is
/// left alone.
#[cfg(unix)]
pub fn reap_stale(pid_file: &Path) {
    let Some(pid) = std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    else {
        let _ = std::fs::remove_file(pid_file);
        return;
    };
    let command = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if command.contains(SERVER_NAME) {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    let _ = std::fs::remove_file(pid_file);
}

/// Windows ties the server to a kill-on-close Job Object instead.
#[cfg(not(unix))]
pub fn reap_stale(_pid_file: &Path) {}

#[cfg(windows)]
mod job {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// A Job Object that kills its processes when its last handle closes,
    /// which includes Purl exiting or crashing.
    pub struct Job(HANDLE);

    // The handle is a kernel object reference, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn kill_on_close() -> io::Result<Job> {
            let handle =
                unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(io::Error::other)?;
            let job = Job(handle);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            }
            .map_err(io::Error::other)?;
            Ok(job)
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            let process = HANDLE(child.as_raw_handle());
            unsafe { AssignProcessToJobObject(self.0, process) }.map_err(io::Error::other)
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_loopback_only_with_thinking_off() {
        let args: Vec<String> = llama_args(Path::new("/m/q.gguf"), 4242)
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        let after = |flag: &str| {
            let i = args.iter().position(|a| a == flag).unwrap();
            args[i + 1].as_str()
        };
        assert_eq!(after("--host"), "127.0.0.1");
        assert_eq!(after("--port"), "4242");
        assert_eq!(after("--reasoning-budget"), "0");
        assert_eq!(after("-m"), "/m/q.gguf");
        assert_eq!(after("-np"), "1");
        assert!(args.contains(&"--offline".to_string()));
        assert!(!args.iter().any(|a| a.contains("0.0.0.0")));
    }

    #[test]
    fn server_path_adds_exe_only_on_windows() {
        let p = llama_server_path(Path::new("/r"));
        if cfg!(windows) {
            assert!(p.ends_with("llama/llama-server.exe"));
        } else {
            assert!(p.ends_with("llama/llama-server"));
        }
    }

    #[test]
    fn free_port_is_bindable_on_loopback() {
        let port = free_port().unwrap();
        assert!(port > 0);
        TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    #[test]
    fn api_keys_are_128_random_bits() {
        let (a, b) = (api_key(), api_key());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn missing_binary_fails_fast() {
        let cmd = Command::new("/nonexistent/llama-server");
        assert!(Sidecar::start(cmd, 1, Duration::from_secs(5), None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_server_that_exits_is_reported_at_once() {
        let t0 = Instant::now();
        let err = Sidecar::start(
            Command::new("false"),
            free_port().unwrap(),
            HEALTH_TIMEOUT,
            None,
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("exited"), "{err}");
        assert!(t0.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn unhealthy_server_times_out_and_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("llama.pid");
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let t0 = Instant::now();
        let err = Sidecar::start(
            cmd,
            free_port().unwrap(),
            Duration::from_millis(300),
            Some(&pid_file),
        )
        .err()
        .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(t0.elapsed() < Duration::from_secs(5));
        assert!(!pid_file.exists(), "drop removes the PID file");
    }

    /// A loopback HTTP server inside this test process. `answer` gets the
    /// request head and returns (status, body). Serves until the process ends.
    fn fake_server(answer: fn(&str) -> (u16, String)) -> u16 {
        use std::io::{Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let (status, body) = answer(&String::from_utf8_lossy(&buf[..n]));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        port
    }

    /// A squatter: says yes to everything, as if it were llama-server.
    fn squatter() -> u16 {
        fake_server(|_| (200, r#"{"model_path":"/m/q.gguf"}"#.into()))
    }

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    /// Behaves like llama-server with API key `KEY` serving `/m/q.gguf`.
    fn honest_server() -> u16 {
        fake_server(|req| {
            if req.contains(&format!("Authorization: Bearer {KEY}"))
                || req.contains(&format!("authorization: Bearer {KEY}"))
            {
                (200, r#"{"model_path":"/m/q.gguf"}"#.into())
            } else {
                (401, r#"{"error":"Invalid API Key"}"#.into())
            }
        })
    }

    #[cfg(unix)]
    #[test]
    fn a_squatter_on_the_port_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("llama.pid");
        let port = squatter();
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let err = Sidecar::start(cmd, port, Duration::from_secs(5), Some(&pid_file))
            .err()
            .expect("a port held by another process must not pass");
        assert!(err.to_string().contains("another program"), "{err}");
        assert!(!pid_file.exists(), "the child was cleaned up");
    }

    #[test]
    fn identity_needs_the_key_and_our_model() {
        let model = Path::new("/m/q.gguf");
        verify_identity(honest_server(), KEY, model).unwrap();
        // Says yes without the key: not llama-server with our key.
        assert!(verify_identity(squatter(), KEY, model).is_err());
        // Right key handling, wrong model.
        assert!(verify_identity(honest_server(), KEY, Path::new("/m/other.gguf")).is_err());
        // Wrong key altogether.
        assert!(verify_identity(honest_server(), &"f".repeat(32), model).is_err());
    }

    #[test]
    fn the_key_never_shows_in_errors() {
        let model = Path::new("/m/q.gguf");
        for port in [squatter(), honest_server(), free_port().unwrap()] {
            if let Err(e) = verify_identity(port, &"e".repeat(32), model) {
                assert!(!e.to_string().contains(&"e".repeat(32)), "{e}");
            }
        }
        let err = verify_identity(squatter(), KEY, model).unwrap_err();
        assert!(!err.to_string().contains(KEY), "{err}");
    }

    /// A child that really owns its port passes, and dies with its Sidecar.
    /// Uses python3 as a stand-in HTTP server.
    #[cfg(unix)]
    #[test]
    fn a_child_that_owns_the_port_is_accepted_and_dies_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("llama.pid");
        let port = free_port().unwrap();
        let mut cmd = Command::new("python3");
        cmd.args([
            "-I",
            "-c",
            "import http.server as h, sys\n\
             class H(h.BaseHTTPRequestHandler):\n \
              def do_GET(s): s.send_response(200); s.end_headers(); s.wfile.write(b'ok')\n \
              def log_message(s, *a): pass\n\
             h.HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()",
            &port.to_string(),
        ]);
        let mut sidecar =
            Sidecar::start(cmd, port, Duration::from_secs(10), Some(&pid_file)).unwrap();
        let pid = sidecar.pid();
        assert!(sidecar.is_alive());
        assert_eq!(std::fs::read_to_string(&pid_file).unwrap(), pid.to_string());
        drop(sidecar);
        assert!(!pid_file.exists());
        assert!(!process_exists(pid), "child {pid} outlived its Sidecar");
    }

    #[cfg(unix)]
    #[test]
    fn reap_leaves_foreign_pids_alone() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("llama.pid");
        let mut other = Command::new("sleep").arg("30").spawn().unwrap();
        std::fs::write(&pid_file, other.id().to_string()).unwrap();
        reap_stale(&pid_file);
        assert!(!pid_file.exists());
        assert!(
            matches!(other.try_wait(), Ok(None)),
            "a sleep isn't ours to kill"
        );
        other.kill().unwrap();
        other.wait().unwrap();
    }

    #[cfg(unix)]
    fn process_exists(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}
