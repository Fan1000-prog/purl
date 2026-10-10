//! Learned corrections: JSONL in the app data dir, capped at CAP.
//! Corrupt lines are skipped on load so one bad write never bricks the store.
//!
//! Same shape as `history.rs` — atomic temp+rename, load-modify-rewrite.

use crate::corrections::{core, is_mishear, Correction};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

pub const CAP: usize = 200;

/// Serializes every load-modify-save on the store. A live watcher's `add`
/// (up to ~30s after a paste) can overlap the next dictation's
/// `record_hits`; without this, both read the file, modify their own copy,
/// and write it back, so whichever wins the rename silently erases the
/// other's update.
///
/// Every caller takes it with `unwrap_or_else(|e| e.into_inner())`: the
/// guard protects no in-memory invariant — each critical section re-reads
/// the file from disk — so a poisoned lock has nothing to protect us from,
/// and refusing to learn corrections for the rest of the session would be
/// the worse outcome.
static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    /// One or more words as dictation produced them.
    pub from: String,
    /// One or more words as the user corrected them.
    pub to: String,
    /// Word that preceded `from` when it was corrected. `^` means text start.
    pub before: String,
    /// Word that followed `from` when it was corrected. Empty means text end.
    pub after: String,
    pub created: u64,
    pub hits: u64,
    pub last_hit: u64,
    /// How many times this exact correction has been observed. Rules that
    /// are not obviously transcription repair (see `safe_from_one_sighting`)
    /// only start applying at 2 — one sighting could be a content edit, two
    /// identical ones are a preference.
    #[serde(default = "default_seen")]
    pub seen: u64,
}

fn default_seen() -> u64 {
    1
}

impl Rule {
    fn same_as(&self, from: &str, to: &str, before: &str) -> bool {
        self.from == from && self.to == to && self.before == before
    }
}

pub fn load(path: &Path) -> Vec<Rule> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

pub fn save(path: &Path, rules: &[Rule]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = String::new();
    for r in rules {
        out.push_str(&serde_json::to_string(r).map_err(std::io::Error::other)?);
        out.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)
}

/// Store a correction. Re-correcting the same word the same way in the same
/// context is not a new rule — it is the user telling us we were right.
pub fn add(path: &Path, c: &Correction, now: u64) -> std::io::Result<()> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rules = load(path);
    if let Some(existing) = rules
        .iter_mut()
        .find(|r| r.same_as(&c.from, &c.to, &c.before))
    {
        existing.seen += 1;
        existing.last_hit = now;
        return save(path, &rules);
    }
    rules.push(Rule {
        from: c.from.clone(),
        to: c.to.clone(),
        before: c.before.clone(),
        after: c.after.clone(),
        created: now,
        hits: 0,
        last_hit: 0,
        seen: 1,
    });
    // Evict the least useful rule, not the oldest: a rule that fires often has
    // earned its slot regardless of age.
    while rules.len() > CAP {
        let worst = rules
            .iter()
            .enumerate()
            .min_by_key(|(_, r)| (r.hits, r.seen, r.last_hit, r.created))
            .map(|(i, _)| i)
            .unwrap_or(0);
        rules.remove(worst);
    }
    save(path, &rules)
}

/// Undo one observation of a correction: the watcher committed it, then the
/// user kept editing and settled on something else (or reverted). A rule
/// observed more than once survives with one sighting fewer; a rule this
/// watch alone created is removed.
pub fn retract(path: &Path, c: &Correction) -> std::io::Result<()> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rules = load(path);
    if let Some(i) = rules
        .iter()
        .position(|r| r.same_as(&c.from, &c.to, &c.before))
    {
        if rules[i].seen > 1 {
            rules[i].seen -= 1;
        } else {
            rules.remove(i);
        }
        return save(path, &rules);
    }
    Ok(())
}

pub fn delete(path: &Path, from: &str, to: &str, before: &str) -> std::io::Result<()> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rules = load(path);
    rules.retain(|r| !r.same_as(from, to, before));
    save(path, &rules)
}

pub fn clear(path: &Path) -> std::io::Result<()> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Rewrite `text` with every rule whose word and context both match.
/// Returns the result and the rules that fired, so hit bookkeeping can
/// happen later — never on the path to the paste.
///
/// Works on byte ranges rather than re-joining tokens: the transcript may
/// already hold newlines (Coding mode markdown, Email paragraphs) and
/// `tokens.join(" ")` would silently flatten them.
pub fn apply(text: &str, rules: &[Rule]) -> (String, Vec<Rule>) {
    // Byte range of every run of non-whitespace, in order.
    let spans: Vec<(usize, usize)> = {
        let mut v = Vec::new();
        let mut start: Option<usize> = None;
        for (i, c) in text.char_indices() {
            match (c.is_whitespace(), start) {
                (false, None) => start = Some(i),
                (true, Some(s)) => {
                    v.push((s, i));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            v.push((s, text.len()));
        }
        v
    };
    let tok = |i: usize| &text[spans[i].0..spans[i].1];

    // Pending edits as (first span, last span — inclusive, replacement).
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut fired: Vec<Rule> = Vec::new();

    for rule in rules {
        if !eligible(rule) {
            continue;
        }
        let from: Vec<&str> = rule.from.split_whitespace().collect();
        if from.is_empty() {
            continue;
        }
        let single = from.len() == 1;
        // A single word with no context at all would be a global
        // substitution — exactly the failure mode this design exists to
        // avoid. A multi-word `from` is its own context.
        if single && rule.before.is_empty() && rule.after.is_empty() {
            continue;
        }
        let mut hit = false;
        for i in 0..spans.len() {
            if i + from.len() > spans.len() {
                break;
            }
            if !(0..from.len()).all(|k| core(tok(i + k)).eq_ignore_ascii_case(core(from[k]))) {
                continue;
            }
            if single {
                let before_ok = match rule.before.as_str() {
                    "" => false,
                    "^" => i == 0,
                    w => i > 0 && core(tok(i - 1)).eq_ignore_ascii_case(w),
                };
                let after_ok = match rule.after.as_str() {
                    "" => false,
                    w => i + 1 < spans.len() && core(tok(i + 1)).eq_ignore_ascii_case(w),
                };
                if !before_ok && !after_ok {
                    continue;
                }
            }
            let last = i + from.len() - 1;
            // First rule to claim a token wins; a later rule must not
            // rewrite a word an earlier one already corrected.
            if edits.iter().any(|(s, e, _)| i <= *e && *s <= last) {
                continue;
            }
            edits.push((i, last, replace_phrase(tok(i), tok(last), &rule.to)));
            hit = true;
        }
        if hit {
            fired.push(rule.clone());
        }
    }

    if edits.is_empty() {
        return (text.to_string(), fired);
    }

    edits.sort_by_key(|(s, _, _)| *s);
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    for (first, last, replacement) in &edits {
        out.push_str(&text[at..spans[*first].0]);
        out.push_str(replacement);
        at = spans[*last].1;
    }
    out.push_str(&text[at..]);
    (out, fired)
}

/// A rule that swaps words for near-identical words — casing, or an edit
/// small relative to the word — is transcription repair and applies from one
/// sighting. Anything else (word choice, phrase reshaping, insertions,
/// deletions) could equally be a one-off content edit, so it waits for a
/// second identical sighting before it may fire.
fn safe_from_one_sighting(rule: &Rule) -> bool {
    let f: Vec<&str> = rule.from.split_whitespace().collect();
    let t: Vec<&str> = rule.to.split_whitespace().collect();
    f.len() == t.len()
        && f.iter()
            .zip(t.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b) || is_mishear(core(a), core(b)))
}

/// Whether a rule may fire yet. Public so the Settings view can tell the
/// user a rule is waiting for its second sighting rather than broken.
pub fn eligible(rule: &Rule) -> bool {
    rule.seen >= 2 || safe_from_one_sighting(rule)
}

/// Swap a phrase's words while keeping the punctuation wrapped around its
/// first and last token. For a single token this preserves both sides.
fn replace_phrase(first: &str, last: &str, to: &str) -> String {
    let fc = core(first);
    let lead = match first.find(fc) {
        Some(at) if !fc.is_empty() => &first[..at],
        _ => "",
    };
    let lc = core(last);
    let trail = match last.find(lc) {
        Some(at) if !lc.is_empty() => &last[at + lc.len()..],
        _ => "",
    };
    format!("{lead}{to}{trail}")
}

/// Corrected words, for the Smart cleanup prompt. They are text read from
/// other apps' fields, so the prompt treats them as data — see
/// `cleanup::build_system_prompt`.
pub fn glossary_terms(rules: &[Rule]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in rules {
        if !out.iter().any(|t| t == &r.to) {
            out.push(r.to.clone());
        }
    }
    out
}

/// Bump the rules that fired. Called from the post-paste bookkeeping block.
///
/// Matches by key rather than by index: the store is re-read here, and the
/// user may have deleted a rule in Settings while the paste was in flight.
pub fn record_hits(path: &Path, fired: &[Rule], now: u64) -> std::io::Result<()> {
    if fired.is_empty() {
        return Ok(());
    }
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rules = load(path);
    for f in fired {
        if let Some(r) = rules
            .iter_mut()
            .find(|r| r.same_as(&f.from, &f.to, &f.before))
        {
            r.hits += 1;
            r.last_hit = now;
        }
    }
    save(path, &rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corrections::Correction;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vt-learned-{name}.jsonl"))
    }

    fn fix(from: &str, to: &str) -> Correction {
        Correction {
            from: from.into(),
            to: to.into(),
            before: "regards".into(),
            after: String::new(),
        }
    }

    #[test]
    fn round_trips_a_rule() {
        let path = tmp("roundtrip");
        let _ = std::fs::remove_file(&path);
        add(&path, &fix("fun", "Fan"), 100).unwrap();
        let rules = load(&path);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].from, "fun");
        assert_eq!(rules[0].to, "Fan");
        assert_eq!(rules[0].created, 100);
        assert_eq!(rules[0].hits, 0);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn re_adding_the_same_correction_bumps_seen_instead_of_duplicating() {
        let path = tmp("dedupe");
        let _ = std::fs::remove_file(&path);
        add(&path, &fix("fun", "Fan"), 100).unwrap();
        add(&path, &fix("fun", "Fan"), 200).unwrap();
        let rules = load(&path);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].seen, 2);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn retract_removes_a_once_seen_rule_and_decrements_a_repeated_one() {
        let path = tmp("retract");
        let _ = std::fs::remove_file(&path);
        add(&path, &fix("fun", "Fan"), 100).unwrap();
        retract(&path, &fix("fun", "Fan")).unwrap();
        assert!(load(&path).is_empty());

        add(&path, &fix("fun", "Fan"), 100).unwrap();
        add(&path, &fix("fun", "Fan"), 200).unwrap();
        retract(&path, &fix("fun", "Fan")).unwrap();
        let rules = load(&path);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].seen, 1);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_rule_without_seen_on_disk_loads_as_seen_once() {
        let path = tmp("legacy");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "{\"from\":\"fun\",\"to\":\"Fan\",\"before\":\"regards\",\"after\":\"\",\"created\":1,\"hits\":0,\"last_hit\":0}\n",
        )
        .unwrap();
        let rules = load(&path);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].seen, 1);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn caps_by_evicting_the_least_hit_rule() {
        let path = tmp("cap");
        let _ = std::fs::remove_file(&path);
        let mut rules: Vec<Rule> = (0..CAP)
            .map(|i| Rule {
                from: format!("w{i}"),
                to: format!("W{i}"),
                before: "^".into(),
                after: String::new(),
                created: i as u64,
                hits: 5,
                last_hit: 0,
                seen: 1,
            })
            .collect();
        rules[7].hits = 0; // the one that has never earned its place
        save(&path, &rules).unwrap();
        add(&path, &fix("fun", "Fan"), 999).unwrap();
        let after = load(&path);
        assert_eq!(after.len(), CAP);
        assert!(!after.iter().any(|r| r.from == "w7"));
        assert!(after.iter().any(|r| r.from == "fun"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn skips_corrupt_lines() {
        let path = tmp("corrupt");
        let _ = std::fs::remove_file(&path);
        add(&path, &fix("fun", "Fan"), 100).unwrap();
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "{{not json").unwrap();
        }
        assert_eq!(load(&path).len(), 1);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn deletes_one_rule_and_leaves_the_rest() {
        let path = tmp("delete");
        let _ = std::fs::remove_file(&path);
        add(&path, &fix("fun", "Fan"), 100).unwrap();
        add(&path, &fix("pid", "PID"), 101).unwrap();
        delete(&path, "fun", "Fan", "regards").unwrap();
        let rules = load(&path);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].from, "pid");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn missing_file_loads_empty_and_clear_is_idempotent() {
        let path = tmp("missing");
        let _ = std::fs::remove_file(&path);
        assert!(load(&path).is_empty());
        clear(&path).unwrap();
        add(&path, &fix("fun", "Fan"), 100).unwrap();
        clear(&path).unwrap();
        assert!(load(&path).is_empty());
    }

    fn rule(from: &str, to: &str, before: &str, after: &str) -> Rule {
        Rule {
            from: from.into(),
            to: to.into(),
            before: before.into(),
            after: after.into(),
            created: 0,
            hits: 0,
            last_hit: 0,
            seen: 1,
        }
    }

    #[test]
    fn fires_when_the_preceding_context_word_matches() {
        let rules = vec![rule("fun", "Fan", "regards", "")];
        let (out, fired) = apply("Regards, fun.", &rules);
        assert_eq!(out, "Regards, Fan.");
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].to, "Fan");
    }

    #[test]
    fn preserves_newlines_and_runs_of_whitespace() {
        // Coding mode emits markdown and Email mode emits paragraphs; a
        // rewrite that re-joined on single spaces would flatten both.
        let rules = vec![rule("fun", "Fan", "regards", "")];
        let (out, _) = apply("- one\n- two\n\nRegards,  fun", &rules);
        assert_eq!(out, "- one\n- two\n\nRegards,  Fan");
    }

    #[test]
    fn returns_the_input_untouched_when_no_rule_fires() {
        let rules = vec![rule("fun", "Fan", "regards", "")];
        let (out, fired) = apply("nothing\tto  see", &rules);
        assert_eq!(out, "nothing\tto  see");
        assert!(fired.is_empty());
    }

    #[test]
    fn leaves_the_word_alone_out_of_context() {
        let rules = vec![rule("fun", "Fan", "regards", "")];
        let (out, fired) = apply("that was fun", &rules);
        assert_eq!(out, "that was fun");
        assert!(fired.is_empty());
    }

    #[test]
    fn fires_on_the_following_context_word() {
        let rules = vec![rule("fun", "Fan", "", "bango")];
        let (out, _) = apply("ask fun bango about it", &rules);
        assert_eq!(out, "ask Fan bango about it");
    }

    #[test]
    fn caret_context_fires_only_at_the_start() {
        let rules = vec![rule("fun", "Fan", "^", "")];
        assert_eq!(apply("fun said hello", &rules).0, "Fan said hello");
        assert_eq!(apply("it was fun", &rules).0, "it was fun");
    }

    #[test]
    fn respects_word_boundaries() {
        let rules = vec![rule("fun", "Fan", "regards", "")];
        assert_eq!(apply("regards funny", &rules).0, "regards funny");
    }

    #[test]
    fn preserves_punctuation_around_the_replaced_word() {
        let rules = vec![rule("fun", "Fan", "regards", "")];
        assert_eq!(apply("Regards, \"fun\"!", &rules).0, "Regards, \"Fan\"!");
    }

    #[test]
    fn a_multi_word_rule_applies_without_context() {
        let mut r = rule("customer voice", "Customer Voice", "the", "tab");
        r.seen = 1; // casing-only per word — safe from one sighting
        let (out, fired) = apply("open customer voice now", &[r]);
        assert_eq!(out, "open Customer Voice now");
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn a_multi_word_rule_keeps_surrounding_punctuation() {
        let rules = vec![rule("customer voice", "Customer Voice", "", "")];
        let (out, _) = apply("(customer voice)", &rules);
        assert_eq!(out, "(Customer Voice)");
    }

    #[test]
    fn a_word_choice_rule_waits_for_a_second_sighting() {
        let mut r = rule("utilize", "use", "should", "");
        assert_eq!(
            apply("we should utilize this", &[r.clone()]).0,
            "we should utilize this"
        );
        r.seen = 2;
        assert_eq!(
            apply("we should utilize this", &[r]).0,
            "we should use this"
        );
    }

    #[test]
    fn a_phrase_reshaping_rule_waits_for_a_second_sighting() {
        let mut r = rule("Regards", "Regards, dear", "^", "fan");
        assert_eq!(apply("Regards Fan", &[r.clone()]).0, "Regards Fan");
        r.seen = 2;
        assert_eq!(apply("Regards Fan", &[r]).0, "Regards, dear Fan");
    }

    #[test]
    fn a_rule_with_no_context_never_fires() {
        // Defensive: an empty-context rule would be a global substitution,
        // which is the failure mode this design exists to avoid.
        let rules = vec![rule("fun", "Fan", "", "")];
        assert_eq!(apply("that was fun", &rules).0, "that was fun");
    }

    #[test]
    fn glossary_terms_are_unique_and_ordered() {
        let rules = vec![
            rule("fun", "Fan", "regards", ""),
            rule("fon", "Fan", "^", ""),
            rule("pid", "PID", "the", ""),
        ];
        assert_eq!(
            glossary_terms(&rules),
            vec!["Fan".to_string(), "PID".to_string()]
        );
    }

    #[test]
    fn record_hits_increments_only_the_rules_that_fired() {
        let path = tmp("hits");
        let _ = std::fs::remove_file(&path);
        save(
            &path,
            &[
                rule("fun", "Fan", "regards", ""),
                rule("pid", "PID", "the", ""),
            ],
        )
        .unwrap();
        record_hits(&path, &[rule("pid", "PID", "the", "")], 555).unwrap();
        let rules = load(&path);
        assert_eq!(rules[0].hits, 0);
        assert_eq!(rules[1].hits, 1);
        assert_eq!(rules[1].last_hit, 555);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn record_hits_ignores_a_rule_deleted_while_the_paste_was_in_flight() {
        let path = tmp("hits-gone");
        let _ = std::fs::remove_file(&path);
        save(&path, &[rule("pid", "PID", "the", "")]).unwrap();
        record_hits(&path, &[rule("fun", "Fan", "regards", "")], 555).unwrap();
        assert_eq!(load(&path).len(), 1);
        std::fs::remove_file(&path).unwrap();
    }

    /// A live watcher's `add` (up to ~30s after a paste) can overlap the next
    /// dictation's `record_hits` on the same store. Without serializing the
    /// load-modify-save, one writer's rename can silently erase the other's
    /// update. This asserts the outcome (nothing lost), not any timing, so it
    /// is deterministic: with the lock in place every one of these adds must
    /// survive regardless of how the threads interleave.
    #[test]
    fn concurrent_writers_do_not_lose_each_others_updates() {
        let path = std::sync::Arc::new(tmp("concurrent"));
        let _ = std::fs::remove_file(&*path);

        const THREADS: usize = 8;
        const PER_THREAD: usize = 10;
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let path = std::sync::Arc::clone(&path);
                std::thread::spawn(move || {
                    for i in 0..PER_THREAD {
                        let c = fix(&format!("w{t}_{i}"), &format!("W{t}_{i}"));
                        add(&path, &c, 1).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(load(&path).len(), THREADS * PER_THREAD);
        std::fs::remove_file(&*path).unwrap();
    }
}
