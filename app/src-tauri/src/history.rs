//! Transcript history: JSONL in the app data dir, newest last, capped at CAP.
//! Corrupt lines are skipped on load so one bad write never bricks history.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CAP: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: u64,
    pub raw: String,
    pub cleaned: String,
    pub used_llm: bool,
    pub record_ms: u64,
    pub transcribe_ms: u64,
    pub cleanup_ms: u64,
    pub words: u64,
}

pub fn load(path: &Path) -> Vec<HistoryEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let entries: Vec<HistoryEntry> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if entries.len() > CAP {
        entries[entries.len() - CAP..].to_vec()
    } else {
        entries
    }
}

/// Load + push + trim + rewrite. At 50 entries a full rewrite is cheap and
/// keeps the file permanently bounded.
pub fn append(path: &Path, entry: &HistoryEntry) -> std::io::Result<()> {
    let mut entries = load(path);
    entries.push(entry.clone());
    if entries.len() > CAP {
        entries.drain(..entries.len() - CAP);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = String::new();
    for e in &entries {
        out.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
        out.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)
}

pub fn clear(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u64) -> HistoryEntry {
        HistoryEntry {
            ts: n,
            raw: format!("raw {n}"),
            cleaned: format!("cleaned {n}"),
            used_llm: n % 2 == 0,
            record_ms: 1000,
            transcribe_ms: 500,
            cleanup_ms: 200,
            words: 2,
        }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vt-history-{name}.jsonl"))
    }

    #[test]
    fn round_trips_entries_in_order() {
        let path = tmp("roundtrip");
        let _ = std::fs::remove_file(&path);
        append(&path, &entry(1)).unwrap();
        append(&path, &entry(2)).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].ts, 1);
        assert_eq!(loaded[1].cleaned, "cleaned 2");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn caps_at_50_keeping_newest() {
        let path = tmp("cap");
        let _ = std::fs::remove_file(&path);
        for n in 0..60 {
            append(&path, &entry(n)).unwrap();
        }
        let loaded = load(&path);
        assert_eq!(loaded.len(), CAP);
        assert_eq!(loaded.first().unwrap().ts, 10);
        assert_eq!(loaded.last().unwrap().ts, 59);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn skips_corrupt_lines() {
        let path = tmp("corrupt");
        append(&path, &entry(1)).unwrap();
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(f, "{{not json").unwrap();
        }
        // Verify that load skips the corrupt line before append rewrites
        let after_corrupt = load(&path);
        assert_eq!(after_corrupt.len(), 1);
        append(&path, &entry(2)).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.len(), 2);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn missing_file_loads_empty_and_clear_is_idempotent() {
        let path = tmp("missing");
        let _ = std::fs::remove_file(&path);
        assert!(load(&path).is_empty());
        clear(&path).unwrap();
        append(&path, &entry(1)).unwrap();
        clear(&path).unwrap();
        assert!(load(&path).is_empty());
    }
}
