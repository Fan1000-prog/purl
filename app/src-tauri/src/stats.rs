//! Cumulative usage counters, JSON in the app data dir. Survives history
//! clearing — these are lifetime totals, not derived from history.jsonl.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// per_day keys kept (YYYY-MM-DD sorts lexicographically = chronologically).
/// 366 covers the Insights "Last 12 months" view with a day to spare.
const PER_DAY_CAP: usize = 366;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub total_words: u64,
    pub total_record_ms: u64,
    pub total_sessions: u64,
    pub llm_cleanups: u64,
    pub per_day: BTreeMap<String, u64>,
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl Stats {
    pub fn load(path: &Path) -> Stats {
        let Ok(content) = std::fs::read_to_string(path) else {
            return Stats::default();
        };
        match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(_) => {
                // Keep the evidence, start fresh.
                let _ = std::fs::rename(path, path.with_extension("json.bak"));
                Stats::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    pub fn record(&mut self, words: u64, record_ms: u64, used_llm: bool, day: &str) {
        self.total_words += words;
        self.total_record_ms += record_ms;
        self.total_sessions += 1;
        if used_llm {
            self.llm_cleanups += 1;
        }
        *self.per_day.entry(day.to_string()).or_insert(0) += words;
        while self.per_day.len() > PER_DAY_CAP {
            let oldest = self.per_day.keys().next().cloned().unwrap();
            self.per_day.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vt-stats-{name}.json"))
    }

    #[test]
    fn record_accumulates() {
        let mut s = Stats::default();
        s.record(10, 6000, true, "2026-07-04");
        s.record(5, 3000, false, "2026-07-04");
        assert_eq!(s.total_words, 15);
        assert_eq!(s.total_record_ms, 9000);
        assert_eq!(s.total_sessions, 2);
        assert_eq!(s.llm_cleanups, 1);
        assert_eq!(s.per_day["2026-07-04"], 15);
    }

    #[test]
    fn per_day_prunes_oldest_past_cap() {
        let mut s = Stats::default();
        for i in 0..370 {
            s.record(1, 100, false, &format!("2026-01-{:03}", i)); // fake but sortable keys
        }
        assert_eq!(s.per_day.len(), 366);
        assert!(!s.per_day.contains_key("2026-01-000"));
        assert!(s.per_day.contains_key("2026-01-369"));
    }

    #[test]
    fn round_trips_through_disk() {
        let path = tmp("roundtrip");
        let mut s = Stats::default();
        s.record(42, 21000, true, "2026-07-04");
        s.save(&path).unwrap();
        let loaded = Stats::load(&path);
        assert_eq!(loaded.total_words, 42);
        assert_eq!(loaded.per_day["2026-07-04"], 42);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn corrupt_file_baks_and_defaults() {
        let path = tmp("corrupt");
        std::fs::write(&path, "{nope").unwrap();
        let loaded = Stats::load(&path);
        assert_eq!(loaded.total_sessions, 0);
        assert!(!path.exists());
        let bak = path.with_extension("json.bak");
        assert!(bak.exists());
        std::fs::remove_file(&bak).unwrap();
    }

    #[test]
    fn today_is_iso_date() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert_eq!(&d[4..5], "-");
    }
}
