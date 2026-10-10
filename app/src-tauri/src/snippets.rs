//! Voice-triggered snippets: say the trigger while dictating and the pipeline
//! replaces it with the expansion before pasting. Pure string matching —
//! deterministic, never the LLM.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub trigger: String,
    pub expansion: String,
}

pub fn load(path: &Path) -> Vec<Snippet> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str(&content) {
        Ok(list) => list,
        Err(_) => {
            let _ = std::fs::rename(path, path.with_extension("json.bak"));
            Vec::new()
        }
    }
}

pub fn save(path: &Path, list: &[Snippet]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(list).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

/// Replace spoken triggers with their expansions.
///
/// Matches on a lowercased copy of `text`; byte offsets are only valid on the
/// original when lowercasing didn't change byte length (true for ASCII and
/// most text). If it did, we conservatively skip expansion rather than risk
/// slicing mid-codepoint.
pub fn expand(text: &str, snippets: &[Snippet]) -> String {
    let lower = text.to_lowercase();
    if lower.len() != text.len() {
        return text.to_string();
    }

    let mut sorted: Vec<&Snippet> = snippets
        .iter()
        .filter(|s| !s.trigger.trim().is_empty())
        .collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.trigger.trim().len()));

    // (start, end, expansion) byte spans on the original text.
    let mut spans: Vec<(usize, usize, String)> = Vec::new();
    for s in &sorted {
        let trig = s.trigger.trim().to_lowercase();
        let mut from = 0;
        while let Some(pos) = lower[from..].find(&trig) {
            let start = from + pos;
            let end = start + trig.len();
            let before_ok = start == 0
                || !lower[..start].chars().next_back().is_some_and(char::is_alphanumeric);
            let after_ok = end == lower.len()
                || !lower[end..].chars().next().is_some_and(char::is_alphanumeric);
            let overlaps = spans.iter().any(|(a, b, _)| start < *b && end > *a);
            if before_ok && after_ok && !overlaps {
                spans.push((start, end, s.expansion.clone()));
            }
            from = end;
        }
    }

    if spans.is_empty() {
        return text.to_string();
    }
    spans.sort_by_key(|(start, _, _)| *start);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, expansion) in spans {
        out.push_str(&text[cursor..start]);
        out.push_str(&expansion);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(trigger: &str, expansion: &str) -> Snippet {
        Snippet {
            trigger: trigger.into(),
            expansion: expansion.into(),
        }
    }

    #[test]
    fn replaces_case_insensitively() {
        let s = [snip("my email address", "you@example.com")];
        assert_eq!(
            expand("Send it to My Email Address please", &s),
            "Send it to you@example.com please"
        );
    }

    #[test]
    fn tolerates_trailing_punctuation() {
        let s = [snip("my email address", "you@example.com")];
        assert_eq!(
            expand("Send it to my email address.", &s),
            "Send it to you@example.com."
        );
    }

    #[test]
    fn respects_word_boundaries() {
        let s = [snip("sig", "Best, Fan")];
        assert_eq!(expand("The design is done", &s), "The design is done");
        assert_eq!(expand("Add sig here", &s), "Add Best, Fan here");
    }

    #[test]
    fn longest_trigger_wins_on_overlap() {
        let s = [snip("email", "SHORT"), snip("my email address", "LONG")];
        assert_eq!(expand("Use my email address now", &s), "Use LONG now");
    }

    #[test]
    fn expansion_is_not_rescanned() {
        // "email" appears inside the first expansion; it must not re-trigger.
        let s = [snip("contact line", "email: you@example.com"), snip("email", "BOOM")];
        assert_eq!(
            expand("Add contact line here", &s),
            "Add email: you@example.com here"
        );
    }

    #[test]
    fn replaces_multiple_occurrences() {
        let s = [snip("brb", "be right back")];
        assert_eq!(expand("brb ok brb", &s), "be right back ok be right back");
    }

    #[test]
    fn non_ascii_lowercase_length_change_is_safe() {
        // 'İ' lowercases to 2 chars (3 bytes); expand must not panic or garble.
        let s = [snip("brb", "be right back")];
        let text = "İstanbul brb";
        assert_eq!(expand(text, &s), text);
    }

    #[test]
    fn empty_triggers_are_ignored() {
        let s = [snip("   ", "x")];
        assert_eq!(expand("hello", &s), "hello");
    }

    #[test]
    fn round_trips_through_disk_and_baks_corrupt() {
        let path = std::env::temp_dir().join("vt-snippets-test.json");
        let list = vec![snip("my email address", "you@example.com")];
        save(&path, &list).unwrap();
        assert_eq!(load(&path).len(), 1);
        std::fs::write(&path, "{bad").unwrap();
        assert!(load(&path).is_empty());
        let bak = path.with_extension("json.bak");
        assert!(bak.exists());
        std::fs::remove_file(&bak).unwrap();
    }
}
