//! A helper process Purl owns: the bundled `llama-server`. It listens on
//! 127.0.0.1 only, on a free port picked at spawn, and dies with the app:
//! `Drop` kills it, a Job Object kills it on Windows if Purl itself dies,
//! and on macOS a PID file lets the next launch reap a leftover.

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
/// page poking at localhost) can't use it. 128 bits from the std hasher's
/// per-instance random seed.
pub fn api_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    let half = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    format!("{:016x}{:016x}", half(), half())
}

pub struct Sidecar {
    child: Child,
    port: u16,
    pid_file: Option<PathBuf>,
    #[cfg(windows)]
    _job: job::Job,
}

impl Sidecar {
    /// Spawn `llama-server` for `model` on a free loopback port and wait
    /// until it is healthy. The API key is passed in the environment, not on
    /// the command line, so it doesn't show in a process list.
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
        Self::start(cmd, port, HEALTH_TIMEOUT, pid_file)
    }

    /// Start `cmd` (which must serve `GET /health` on `port`) and wait up to
    /// `health_timeout` for a 200. The child is killed on any failure.
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
    fn api_keys_differ() {
        let (a, b) = (api_key(), api_key());
        assert_eq!(a.len(), 32);
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

    /// A loopback HTTP server that answers every request 200, standing in
    /// for a healthy llama-server. Serves until the listener is dropped.
    #[cfg(unix)]
    fn healthy_port() -> (TcpListener, u16) {
        use std::io::{Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accept = listener.try_clone().unwrap();
        std::thread::spawn(move || {
            for mut stream in accept.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            }
        });
        (listener, port)
    }

    #[cfg(unix)]
    #[test]
    fn healthy_sidecar_writes_pid_and_dies_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("llama.pid");
        let (_server, port) = healthy_port();
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let mut sidecar =
            Sidecar::start(cmd, port, Duration::from_secs(5), Some(&pid_file)).unwrap();
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
