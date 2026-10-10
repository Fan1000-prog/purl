//! End-to-end tests for `engine::download` against a local HTTP fixture
//! server. Nothing here touches the network: the server binds 127.0.0.1 and
//! serves a 1 MiB in-memory file, honouring `Range`, with scripted failures.

use purl_lib::engine::download::{download_model, DownloadError};
use purl_lib::engine::models::{self, Kind, ModelFile, ModelSpec};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const FIXTURE_LEN: usize = 1024 * 1024;
const CUT_AT: usize = 400 * 1024;
const OVERSIZED_COPIES: usize = 16;
/// Bytes the `Oversized` reply got onto the wire. Only one test uses it.
static OVERSIZED_SENT: AtomicUsize = AtomicUsize::new(0);

/// What the server does with the next request. Unscripted requests get `Serve`.
#[derive(Clone, Copy)]
enum Reply {
    Serve,
    Status(u16),
    /// Promise the full body, send this many bytes, then hang up.
    CloseAfter(usize),
    /// Stream the file over and over, up to `OVERSIZED_COPIES` times, with no
    /// Content-Length: a server serving far more than the registry says.
    Oversized,
}

struct Fixture {
    url: String,
    data: Arc<Vec<u8>>,
    script: Arc<Mutex<VecDeque<Reply>>>,
    /// The `Range` header of every request, `None` when absent.
    ranges: Arc<Mutex<Vec<Option<String>>>>,
}

impl Fixture {
    fn start() -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let data: Arc<Vec<u8>> = Arc::new((0..FIXTURE_LEN).map(|i| (i * 31 % 251) as u8).collect());
        let script = Arc::new(Mutex::new(VecDeque::new()));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let (d, s, r) = (data.clone(), script.clone(), ranges.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let reply = s.lock().unwrap().pop_front().unwrap_or(Reply::Serve);
                serve(stream, &d, reply, &r);
            }
        });
        Fixture {
            url: format!("http://127.0.0.1:{port}/f.bin"),
            data,
            script,
            ranges,
        }
    }

    fn then(&self, reply: Reply) {
        self.script.lock().unwrap().push_back(reply);
    }

    fn sha(&self) -> String {
        hex(&Sha256::digest(self.data.as_slice()))
    }

    fn ranges(&self) -> Vec<Option<String>> {
        self.ranges.lock().unwrap().clone()
    }
}

fn serve(
    mut stream: std::net::TcpStream,
    data: &[u8],
    reply: Reply,
    ranges: &Mutex<Vec<Option<String>>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            }
        }
    }
    ranges.lock().unwrap().push(range.clone());

    let start = range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
    let (status, body) = match (reply, start) {
        (Reply::Status(code), _) => {
            let _ = write!(
                stream,
                "HTTP/1.1 {code} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            return;
        }
        (_, Some(s)) => (
            format!(
                "206 Partial Content\r\nContent-Range: bytes {s}-{}/{}",
                data.len() - 1,
                data.len()
            ),
            &data[s..],
        ),
        (_, None) => ("200 OK".to_string(), data),
    };
    if let Reply::Oversized = reply {
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
        // Bounded so a client without a size cap fails the test instead of
        // filling the disk. Stops early once the client hangs up.
        for chunk in data.chunks(64 * 1024).cycle().take(OVERSIZED_COPIES * 16) {
            if stream.write_all(chunk).is_err() {
                break;
            }
            OVERSIZED_SENT.fetch_add(chunk.len(), Ordering::SeqCst);
        }
        return;
    }
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let sent = match reply {
        Reply::CloseAfter(n) => &body[..n.min(body.len())],
        _ => body,
    };
    let _ = stream.write_all(sent);
    let _ = stream.flush();
    // Dropping the stream hangs up, mid-body for `CloseAfter`.
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A fresh, empty app-data dir per test.
fn app_data() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "purl-dl-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A one-file spec pointing at `url`. Leaked so it matches the registry's
/// `'static` shape — fine in a test process.
fn spec(url: &str, size: u64, sha256: &str) -> &'static ModelSpec {
    let files: &'static [ModelFile] = Box::leak(Box::new([ModelFile {
        name: "f.bin",
        url: Box::leak(url.to_string().into_boxed_str()),
        size,
        sha256: Box::leak(sha256.to_string().into_boxed_str()),
    }]));
    Box::leak(Box::new(ModelSpec {
        id: "test-model",
        label: "Test model",
        kind: Kind::Stt,
        files,
        licence: "MIT",
    }))
}

fn file_in(app_data: &Path, name: &str) -> PathBuf {
    models::model_dir(app_data, "test-model").join(name)
}

fn run(spec: &ModelSpec, app_data: &Path) -> Result<(), DownloadError> {
    download_model(spec, app_data, &AtomicBool::new(false), &mut |_| {})
}

#[test]
fn downloads_and_verifies() {
    let fx = Fixture::start();
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &fx.sha());

    assert_eq!(run(spec, &dir), Ok(()));

    assert_eq!(std::fs::read(file_in(&dir, "f.bin")).unwrap(), *fx.data);
    assert!(!file_in(&dir, "f.bin.part").exists());
    assert!(models::is_installed(&dir, spec));
}

#[test]
fn installed_model_is_not_fetched_again() {
    let fx = Fixture::start();
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &fx.sha());
    run(spec, &dir).unwrap();

    assert_eq!(run(spec, &dir), Ok(()));
    assert_eq!(fx.ranges().len(), 1, "second run must not hit the server");
}

#[test]
fn checksum_mismatch_deletes_file() {
    let fx = Fixture::start();
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &"0".repeat(64));

    assert_eq!(run(spec, &dir), Err(DownloadError::Checksum));

    assert!(!file_in(&dir, "f.bin").exists());
    assert!(!file_in(&dir, "f.bin.part").exists());
    assert!(!models::is_installed(&dir, spec));
}

#[test]
fn http_404_is_reported() {
    let fx = Fixture::start();
    fx.then(Reply::Status(404));
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &fx.sha());

    assert_eq!(run(spec, &dir), Err(DownloadError::Http(404)));
}

#[test]
fn resume_after_restart_uses_part_file() {
    let fx = Fixture::start();
    fx.then(Reply::CloseAfter(CUT_AT));
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &fx.sha());

    let first = run(spec, &dir);
    assert!(
        matches!(
            first,
            Err(DownloadError::Offline) | Err(DownloadError::Io(_))
        ),
        "got {first:?}"
    );
    let part = file_in(&dir, "f.bin.part");
    assert_eq!(std::fs::metadata(&part).unwrap().len(), CUT_AT as u64);

    // A relaunch is just a fresh call: nothing survives but the .part file.
    assert_eq!(run(spec, &dir), Ok(()));

    assert_eq!(fx.ranges(), vec![None, Some(format!("bytes={CUT_AT}-"))]);
    assert_eq!(std::fs::read(file_in(&dir, "f.bin")).unwrap(), *fx.data);
    assert!(!part.exists());
}

#[test]
fn range_not_satisfiable_restarts_from_scratch() {
    let fx = Fixture::start();
    fx.then(Reply::Status(416));
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &fx.sha());
    let part = file_in(&dir, "f.bin.part");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, vec![7u8; CUT_AT]).unwrap();

    assert_eq!(run(spec, &dir), Ok(()));

    assert_eq!(fx.ranges(), vec![Some(format!("bytes={CUT_AT}-")), None]);
    assert_eq!(std::fs::read(file_in(&dir, "f.bin")).unwrap(), *fx.data);
}

#[test]
fn body_longer_than_registry_size_is_cut_off() {
    let fx = Fixture::start();
    fx.then(Reply::Oversized);
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &fx.sha());
    let mut most = 0;

    let result = download_model(spec, &dir, &AtomicBool::new(false), &mut |p| {
        most = most.max(p.downloaded)
    });

    assert_eq!(result, Err(DownloadError::Checksum));
    assert!(most <= FIXTURE_LEN as u64, "reported {most} bytes");
    // The client hung up early instead of reading all 16 MiB.
    let sent = OVERSIZED_SENT.load(Ordering::SeqCst);
    assert!(
        sent < OVERSIZED_COPIES * FIXTURE_LEN,
        "server sent {sent} bytes"
    );
    assert!(!file_in(&dir, "f.bin.part").exists());
    assert!(!file_in(&dir, "f.bin").exists());
}

#[test]
fn panic_mid_download_becomes_an_error() {
    let fx = Fixture::start();
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &fx.sha());

    let result = download_model(spec, &dir, &AtomicBool::new(false), &mut |_| {
        panic!("progress sink blew up")
    });

    assert_eq!(result, Err(DownloadError::Stopped));
    assert_eq!(
        DownloadError::Stopped.to_string(),
        "The download stopped unexpectedly. Press Retry."
    );
}

#[test]
fn cancel_stops_and_keeps_part() {
    let fx = Fixture::start();
    let dir = app_data();
    let spec = spec(&fx.url, FIXTURE_LEN as u64, &fx.sha());
    let cancel = AtomicBool::new(false);

    let result = download_model(spec, &dir, &cancel, &mut |_| {
        cancel.store(true, Ordering::SeqCst)
    });

    assert_eq!(result, Err(DownloadError::Cancelled));
    let kept = std::fs::metadata(file_in(&dir, "f.bin.part"))
        .unwrap()
        .len();
    assert!(kept > 0 && kept < FIXTURE_LEN as u64, "kept {kept} bytes");
    assert!(!file_in(&dir, "f.bin").exists());
}

#[test]
fn refuses_when_disk_too_small() {
    let fx = Fixture::start();
    let dir = app_data();
    let spec = spec(&fx.url, u64::MAX / 2, &fx.sha());

    assert!(matches!(
        run(spec, &dir),
        Err(DownloadError::DiskFull { .. })
    ));
    assert!(fx.ranges().is_empty(), "no request before the space check");
}

#[test]
fn connection_refused_is_offline() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dir = app_data();
    let spec = spec(
        &format!("http://127.0.0.1:{port}/f.bin"),
        FIXTURE_LEN as u64,
        &"0".repeat(64),
    );

    assert_eq!(run(spec, &dir), Err(DownloadError::Offline));
}

#[test]
fn every_registry_entry_has_sha_and_https() {
    for m in models::registry() {
        for f in m.files {
            assert_eq!(f.sha256.len(), 64, "{}/{}", m.id, f.name);
            assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(f.url.starts_with("https://"), "{}", f.url);
            assert!(f.size > 0);
        }
    }
}
