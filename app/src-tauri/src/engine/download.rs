//! Verified, resumable model downloads.
//!
//! Each file streams into `<file>.part` and only becomes `<file>` once its
//! sha256 matches the registry. An interrupted download keeps its `.part`,
//! and the next attempt resumes it with an HTTP `Range` request.

use super::models::{is_file_verified, model_dir, ok_marker, ModelFile, ModelSpec};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DownloadError {
    #[error("Not enough free space: need {need_mb} MB, have {have_mb} MB.")]
    DiskFull { need_mb: u64, have_mb: u64 },
    #[error("You're offline. Connect to the internet and press Retry.")]
    Offline,
    #[error("The download server said {0}. Press Retry in a minute.")]
    Http(u16),
    #[error("The file didn't match its fingerprint, so it was deleted. Press Retry.")]
    Checksum,
    #[error("Download cancelled.")]
    Cancelled,
    #[error("Couldn't save the file: {0}")]
    Io(String),
}

impl From<std::io::Error> for DownloadError {
    fn from(e: std::io::Error) -> Self {
        DownloadError::Io(e.to_string())
    }
}

/// Bytes downloaded so far across the whole model, out of its total size.
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
}

const CHUNK: usize = 256 * 1024;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const MB: u64 = 1024 * 1024;

/// Fetch every file of `spec` into `model_dir(app_data, spec.id)`, skipping
/// files already verified. Blocking; run it off the main thread.
pub fn download_model(
    spec: &ModelSpec,
    app_data: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), DownloadError> {
    let dir = model_dir(app_data, spec.id);
    fs::create_dir_all(&dir)?;

    let mut pending = Vec::new();
    for file in spec.files {
        if !verify_existing(&dir, file)? {
            pending.push(file);
        }
    }
    if pending.is_empty() {
        return Ok(());
    }

    // Refuse up front rather than fill the disk halfway through 650 MB.
    // The 10% margin leaves room for everything else on the machine.
    let remaining = pending
        .iter()
        .map(|f| f.size.saturating_sub(part_len(&dir, f)))
        .fold(0u64, u64::saturating_add);
    let need = remaining.saturating_add(remaining / 10);
    let have = free_space(&dir);
    if have < need {
        return Err(DownloadError::DiskFull {
            need_mb: need.div_ceil(MB),
            have_mb: have / MB,
        });
    }

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // Blocking reqwest applies this per send and per body read, not to
        // the whole transfer, so it acts as a stall timeout.
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| DownloadError::Io(e.to_string()))?;

    let total = spec.size();
    let mut done: u64 = total - pending.iter().map(|f| f.size).sum::<u64>();
    let mut last_report: Option<Instant> = None;
    for file in pending {
        fetch_file(&client, &dir, file, cancel, &mut |file_bytes| {
            if last_report.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                last_report = Some(Instant::now());
                on_progress(Progress {
                    downloaded: done + file_bytes,
                    total,
                });
            }
        })?;
        done += file.size;
    }
    Ok(())
}

/// Free bytes on the volume holding `path`; 0 when it can't be read, so an
/// unreadable volume refuses the download instead of filling up.
pub fn free_space(path: &Path) -> u64 {
    fs4::available_space(path).unwrap_or(0)
}

/// True when `file` is already in place and verified. A final file of the
/// right size without its marker (say, the app quit between rename and
/// marker write) is hashed once instead of fetched again.
fn verify_existing(dir: &Path, file: &ModelFile) -> Result<bool, DownloadError> {
    if is_file_verified(dir, file) {
        return Ok(true);
    }
    let path = dir.join(file.name);
    let Ok(meta) = fs::metadata(&path) else {
        return Ok(false);
    };
    if meta.len() == file.size && sha256_hex(&path)? == file.sha256 {
        fs::write(ok_marker(dir, file), file.sha256)?;
        return Ok(true);
    }
    fs::remove_file(&path)?;
    Ok(false)
}

fn part_len(dir: &Path, file: &ModelFile) -> u64 {
    fs::metadata(dir.join(format!("{}.part", file.name)))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Stream one file into its `.part`, resuming if one exists, then verify and
/// move it into place. `on_bytes` gets this file's byte count after each chunk.
fn fetch_file(
    client: &reqwest::blocking::Client,
    dir: &Path,
    file: &ModelFile,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<(), DownloadError> {
    let part = dir.join(format!("{}.part", file.name));
    let mut have = part_len(dir, file);
    if have > file.size {
        // Longer than the real file: can't be a prefix of it.
        fs::remove_file(&part)?;
        have = 0;
    }

    if have < file.size {
        let mut request = client.get(file.url);
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let mut response = request.send().map_err(network_error)?;
        let status = response.status();
        let mut out = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            OpenOptions::new().append(true).create(true).open(&part)?
        } else if status.is_success() {
            // The server ignored the Range header and sent the whole file.
            have = 0;
            File::create(&part)?
        } else {
            return Err(DownloadError::Http(status.as_u16()));
        };

        let mut buf = vec![0u8; CHUNK];
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(DownloadError::Cancelled);
            }
            // A dropped connection mid-body is a network problem, not a disk
            // one; the .part keeps what arrived so Retry resumes from there.
            let n = response
                .read(&mut buf)
                .map_err(|_| DownloadError::Offline)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            have += n as u64;
            on_bytes(have);
        }
        out.sync_all()?;
        if have < file.size {
            return Err(DownloadError::Offline);
        }
    }

    if sha256_hex(&part)? != file.sha256 {
        fs::remove_file(&part)?;
        return Err(DownloadError::Checksum);
    }
    fs::rename(&part, dir.join(file.name))?;
    fs::write(ok_marker(dir, file), file.sha256)?;
    Ok(())
}

fn network_error(e: reqwest::Error) -> DownloadError {
    if e.is_connect() || e.is_timeout() || e.is_request() {
        DownloadError::Offline
    } else {
        DownloadError::Io(e.to_string())
    }
}

fn sha256_hex(path: &Path) -> Result<String, DownloadError> {
    let mut hasher = Sha256::new();
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_user_facing_sentences() {
        assert_eq!(
            DownloadError::DiskFull {
                need_mb: 720,
                have_mb: 100
            }
            .to_string(),
            "Not enough free space: need 720 MB, have 100 MB."
        );
        assert_eq!(
            DownloadError::Http(503).to_string(),
            "The download server said 503. Press Retry in a minute."
        );
    }

    #[test]
    fn sha256_hex_matches_known_digest() {
        let path = std::env::temp_dir().join(format!("purl-sha-{}", std::process::id()));
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_hex(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = fs::remove_file(&path);
    }
}
