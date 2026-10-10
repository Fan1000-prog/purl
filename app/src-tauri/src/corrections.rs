//! Pure correction detection: given the text we pasted and what the field
//! holds now, decide what the user changed by hand and whether it is worth
//! learning.
//!
//! Nothing here touches the OS, the clock, or the disk. The whole feature's
//! judgement lives in this module so it can be tested exhaustively.
//!
//! The detector aligns the pasted text against the field word-by-word and
//! extracts every contiguous changed run — substitutions, insertions and
//! deletions, one word or several. What separates a correction from a
//! content edit is not the *kind* of change (the old single-word Levenshtein
//! gate) but its shape and its repetition: localized edits are learned here,
//! and `learned::apply` holds risky rules back until the same correction has
//! been seen twice.

/// One hand-made edit, with the words that surrounded it.
#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    /// One or more words as we pasted them.
    pub from: String,
    /// One or more words as the user left them.
    pub to: String,
    /// Word before `from` at correction time, lowercased. `^` at text start.
    pub before: String,
    /// Word after `from` at correction time, lowercased. Empty at text end.
    pub after: String,
}

/// A token with its surrounding punctuation removed. Interior punctuation
/// stays, so "well-known" and "don't" survive intact.
pub fn core(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric())
}

/// Case-insensitive Levenshtein distance, computed in full over two rolling
/// rows. No early exit: callers compare single words, where the full matrix
/// is a handful of cells.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// True when `to` is plausibly Whisper mis-hearing `from`: same word in a
/// different case, or a small edit relative to the word's length. No longer a
/// gate on what gets *learned* — it decides which rules are safe to *apply*
/// after a single sighting (see `learned`).
pub(crate) fn is_mishear(from: &str, to: &str) -> bool {
    if from.is_empty() || to.is_empty() || from == to {
        return false;
    }
    if from.eq_ignore_ascii_case(to) {
        return true;
    }
    let d = distance(from, to);
    let longer = from.chars().count().max(to.chars().count());
    d <= 2 && d * 2 <= longer
}

/// Characters that apps rewrite on their own (smart quotes, NBSP). A diff
/// that disappears under this mapping was made by the target app, not the
/// user, and must not be learned.
fn normalize_char(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' => '\'',
        '\u{201C}' | '\u{201D}' => '"',
        '\u{00A0}' => ' ',
        _ => c,
    }
}

fn normalize_word(s: &str) -> String {
    s.chars().map(normalize_char).collect()
}

/// A changed run longer than this on either side is a rewrite of that
/// passage, not a fix.
const MAX_RUN_WORDS: usize = 6;
/// More distinct edit sites than this means the user reworked the text.
const MAX_RUNS: usize = 3;
/// A replacement may grow by at most this many words over what it replaces.
const MAX_RUN_GROWTH: usize = 2;

/// Compare what we pasted against what the field holds now and return every
/// learnable edit. Empty when nothing changed or the change reads as a
/// rewrite rather than a correction.
pub fn detect_edits(pasted: &str, current: &str) -> Vec<Correction> {
    let want: Vec<&str> = pasted.split_whitespace().collect();
    let have: Vec<&str> = current.split_whitespace().collect();
    let (n, m) = (want.len(), have.len());
    if n == 0 || m == 0 || n * m > 512 * 512 {
        return Vec::new();
    }

    // LCS on raw tokens — case-sensitive, because casing fixes are real
    // corrections and must show up as changes.
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if want[i] == have[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    // Walk the alignment, grouping consecutive non-matches into runs of
    // (want_start, want_end, have_start, have_end).
    let mut runs: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if want[i] == have[j] {
            if let Some((ws, hs)) = open.take() {
                runs.push((ws, i, hs, j));
            }
            i += 1;
            j += 1;
        } else {
            if open.is_none() {
                open = Some((i, j));
            }
            if dp[i + 1][j] >= dp[i][j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    if i < n || j < m {
        let (ws, hs) = open.take().unwrap_or((i, j));
        runs.push((ws, n, hs, m));
    } else if let Some((ws, hs)) = open.take() {
        runs.push((ws, i, hs, j));
    }

    // Words the user typed *after* our text are normal typing, not a
    // correction. A pure insertion at the very end is dropped; a trailing
    // substitution keeps only as many replacement words as it replaces —
    // the correction — and leaves the typed continuation alone.
    if let Some(last) = runs.last_mut() {
        let (ws, we, hs, he) = *last;
        if ws == we && we == n && he == m {
            runs.pop();
        } else if we == n && he == m && he - hs > we - ws {
            // A trailing substitution that grew: indistinguishable from the
            // user fixing the last words and then typing onward. Keep the
            // positional replacement, drop the growth — mid-text expansions
            // still learn in full, where the following words disambiguate.
            last.3 = hs + (we - ws);
        }
    }

    if runs.is_empty() || runs.len() > MAX_RUNS {
        return Vec::new();
    }
    let total_changed: usize = runs
        .iter()
        .map(|&(ws, we, hs, he)| (we - ws).max(he - hs))
        .sum();
    if total_changed > 3.max(n / 2) {
        return Vec::new(); // most of the text moved — a rewrite, not fixes
    }

    let mut out = Vec::new();
    for &(ws, we, hs, he) in &runs {
        // Fold a neighbouring common word into pure insertions and deletions
        // so every rule is a non-empty from→to substitution with a place to
        // land when applied.
        let (fs, fe, gs, ge) = if ws == we {
            // insertion
            if ws > 0 {
                (ws - 1, we, hs - 1, he)
            } else if we < n {
                (ws, we + 1, hs, he + 1)
            } else {
                continue;
            }
        } else if hs == he {
            // deletion
            if ws > 0 {
                (ws - 1, we, hs - 1, he)
            } else if we < n {
                (ws, we + 1, hs, he + 1)
            } else {
                continue;
            }
        } else {
            (ws, we, hs, he)
        };
        if fe - fs > MAX_RUN_WORDS || ge - gs > MAX_RUN_WORDS {
            continue;
        }
        if ge - gs > (fe - fs) + MAX_RUN_GROWTH {
            continue;
        }

        let trim = |words: &[&str]| -> String {
            words
                .join(" ")
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        };
        let from = trim(&want[fs..fe]);
        let to = trim(&have[gs..ge]);
        if from.is_empty() || to.is_empty() || from == to {
            continue;
        }
        // The target app's own substitutions (smart quotes) are not the user.
        if normalize_word(&from) == normalize_word(&to) {
            continue;
        }
        // A digit on either side reads as content — a time, a date, a
        // quantity the user changed their mind about — never transcription.
        if from.chars().any(|c| c.is_ascii_digit()) || to.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }

        out.push(Correction {
            from,
            to,
            before: if fs == 0 {
                "^".to_string()
            } else {
                core(want[fs - 1]).to_lowercase()
            },
            after: want
                .get(fe)
                .map(|w| core(w).to_lowercase())
                .unwrap_or_default(),
        });
    }
    out
}

/// One reading of the focused text field. Plain owned data so every layer
/// above the FFI can be tested without it.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Stable-enough identity for "still the same field": pid, role, title.
    pub id: String,
    pub value: String,
    /// True for password fields, which are never watched.
    pub secure: bool,
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Continue,
    /// Stop watching. The reason is a fixed string safe to log — it never
    /// contains text read from another application.
    Abandon(&'static str),
    /// The stable set of corrections changed. The previous set (if any) is
    /// superseded: the caller retracts it and stores this one. An empty set
    /// means the user reverted their edit after we had learned it.
    Learned(Vec<Correction>),
}

/// Normalized copy of `s` (app-substitution chars mapped, whitespace runs
/// collapsed to single spaces), with a map from each normalized byte offset
/// back to the raw byte offset it came from.
fn normalized_with_map(s: &str) -> (String, Vec<(usize, usize)>) {
    let mut out = String::new();
    let mut map = Vec::new();
    let mut last_ws = false;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() || c == '\u{00A0}' {
            if !last_ws {
                map.push((out.len(), i));
                out.push(' ');
                last_ws = true;
            }
        } else {
            map.push((out.len(), i));
            out.push(normalize_char(c));
            last_ws = false;
        }
    }
    (out, map)
}

fn raw_offset(map: &[(usize, usize)], norm: usize, raw_len: usize) -> usize {
    match map.binary_search_by_key(&norm, |&(nb, _)| nb) {
        Ok(k) => map[k].1,
        Err(k) if k >= map.len() => raw_len,
        Err(k) => map[k].1,
    }
}

/// Locate the pasted text in the field: exact match first, then normalized
/// (smart quotes, collapsed whitespace) so apps that rewrite what they
/// receive don't refuse the watch. The *last* occurrence wins — it is the
/// one we just pasted.
fn find_span(value: &str, pasted: &str) -> Option<(usize, usize)> {
    if let Some(at) = value.rfind(pasted) {
        return Some((at, at + pasted.len()));
    }
    let (nvalue, map) = normalized_with_map(value);
    let (npasted, _) = normalized_with_map(pasted);
    let npasted = npasted.trim();
    if npasted.is_empty() {
        return None;
    }
    let at = nvalue.rfind(npasted)?;
    Some((
        raw_offset(&map, at, value.len()),
        raw_offset(&map, at + npasted.len(), value.len()),
    ))
}

/// How much of the surrounding text anchors the span when the exact prefix
/// no longer matches (the user edited something earlier in the document).
const ANCHOR_CHARS: usize = 24;

fn tail_chars(s: &str, n: usize) -> &str {
    match s.char_indices().rev().nth(n - 1) {
        Some((i, _)) => &s[i..],
        None => s,
    }
}

fn head_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Tracks one pasted span while the user edits around it.
pub struct Watch {
    id: String,
    prefix: String,
    suffix: String,
    /// The span as the field held it right after the paste — the baseline
    /// edits are measured against. Field content, not our clipboard string,
    /// so an app's own rewriting (smart quotes) never reads as a user edit.
    pasted: String,
    /// The correction set seen on the previous poll. A set is only committed
    /// once two consecutive polls agree, so partial typing ("Fa") never gets
    /// learned — and if it does slip through, the next stable state retracts
    /// it (see `Step::Learned`).
    pending: Option<Vec<Correction>>,
    /// The set most recently committed via `Step::Learned`.
    committed: Vec<Correction>,
}

impl Watch {
    /// Locate the pasted text in the field as it stands immediately after
    /// the paste. Unreadable or secure fields refuse the watch outright.
    pub fn start(first: &Snapshot, pasted: &str) -> Result<Watch, &'static str> {
        if first.secure {
            return Err("secure field");
        }
        if pasted.is_empty() {
            return Err("nothing pasted");
        }
        let (at, end) = find_span(&first.value, pasted).ok_or("pasted text not found")?;
        Ok(Watch {
            id: first.id.clone(),
            prefix: first.value[..at].to_string(),
            suffix: first.value[end..].to_string(),
            pasted: first.value[at..end].to_string(),
            pending: None,
            committed: Vec::new(),
        })
    }

    /// Find our span in the current value when the exact prefix no longer
    /// matches: anchor on the last ANCHOR_CHARS of the prefix and the first
    /// ANCHOR_CHARS of the suffix, so edits elsewhere in the document don't
    /// end the watch. An edit *inside* the near anchor genuinely loses the
    /// span, and the watch gives up.
    fn reanchor<'a>(&self, value: &'a str) -> Option<&'a str> {
        let start = if self.prefix.is_empty() {
            0
        } else {
            let anchor = tail_chars(&self.prefix, ANCHOR_CHARS);
            value.rfind(anchor)? + anchor.len()
        };
        let end = if self.suffix.is_empty() {
            value.len()
        } else {
            let anchor = head_chars(&self.suffix, ANCHOR_CHARS);
            match value[start..].find(anchor) {
                Some(i) => start + i,
                None => value.len(), // suffix is a hint; typed-over is fine
            }
        };
        (start <= end).then(|| &value[start..end])
    }

    pub fn step(&mut self, now: &Snapshot) -> Step {
        if now.id != self.id {
            return Step::Abandon("focus changed");
        }
        if now.secure {
            return Step::Abandon("secure field");
        }
        let span = if let Some(rest) = now.value.strip_prefix(&self.prefix) {
            rest.strip_suffix(&self.suffix).unwrap_or(rest)
        } else {
            match self.reanchor(&now.value) {
                Some(s) => s,
                None => return Step::Abandon("edited outside span"),
            }
        };

        let seen = detect_edits(&self.pasted, span);
        if self.pending.as_ref() == Some(&seen) {
            if seen != self.committed {
                self.committed = seen.clone();
                return Step::Learned(seen);
            }
            Step::Continue
        } else {
            self.pending = Some(seen);
            Step::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(pasted: &str, current: &str) -> Correction {
        let mut v = detect_edits(pasted, current);
        assert_eq!(v.len(), 1, "expected exactly one correction: {v:?}");
        v.remove(0)
    }

    #[test]
    fn learns_a_single_word_substitution() {
        let c = one("Regards, fun.", "Regards, Fan.");
        assert_eq!(c.from, "fun");
        assert_eq!(c.to, "Fan");
        assert_eq!(c.before, "regards");
        assert_eq!(c.after, "");
    }

    #[test]
    fn learns_a_casing_only_substitution() {
        let c = one("thanks fan", "thanks Fan");
        assert_eq!(c.from, "fan");
        assert_eq!(c.to, "Fan");
    }

    #[test]
    fn records_caret_as_context_at_the_start() {
        let c = one("fun is here", "Fan is here");
        assert_eq!(c.before, "^");
        assert_eq!(c.after, "is");
    }

    #[test]
    fn learns_a_multi_word_substitution() {
        let c = one("open the customer voice tab", "open the Customer Voice tab");
        assert_eq!(c.from, "customer voice");
        assert_eq!(c.to, "Customer Voice");
        assert_eq!(c.before, "the");
        assert_eq!(c.after, "tab");
    }

    #[test]
    fn learns_a_word_choice_change() {
        let c = one("we should utilize this", "we should use this");
        assert_eq!(c.from, "utilize");
        assert_eq!(c.to, "use");
    }

    #[test]
    fn learns_an_insertion_by_folding_the_left_neighbour() {
        let c = one("Regards, Fan", "Regards, dear Fan");
        assert_eq!(c.from, "Regards");
        assert_eq!(c.to, "Regards, dear");
        assert_eq!(c.before, "^");
        assert_eq!(c.after, "fan");
    }

    #[test]
    fn learns_a_deletion_by_folding_the_left_neighbour() {
        let c = one("so um the report", "so the report");
        assert_eq!(c.from, "so um");
        assert_eq!(c.to, "so");
    }

    #[test]
    fn learns_two_separate_fixes_as_two_corrections() {
        let v = detect_edits("teh report from fun", "the report from Fan");
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].from.as_str(), v[0].to.as_str()), ("teh", "the"));
        assert_eq!((v[1].from.as_str(), v[1].to.as_str()), ("fun", "Fan"));
    }

    #[test]
    fn rejects_a_full_rewrite() {
        assert!(detect_edits(
            "send the report to the whole team today",
            "actually let us postpone everything until next week"
        )
        .is_empty());
    }

    #[test]
    fn rejects_a_digit_change_as_content() {
        assert!(detect_edits("meet at 3 pm", "meet at 4 pm").is_empty());
    }

    #[test]
    fn rejects_a_punctuation_only_change() {
        assert!(detect_edits("Regards, Fan", "Regards, Fan!").is_empty());
    }

    #[test]
    fn rejects_a_smart_quote_substitution_by_the_app() {
        assert!(detect_edits("don't stop", "don\u{2019}t stop").is_empty());
    }

    #[test]
    fn ignores_words_typed_after_our_text() {
        let c = one("Regards, fun", "Regards, Fan and see you Tuesday");
        assert_eq!(c.from, "fun");
        assert_eq!(c.to, "Fan");
    }

    #[test]
    fn ignores_a_pure_continuation_at_the_end() {
        assert!(detect_edits("Regards, Fan", "Regards, Fan see you Tuesday").is_empty());
    }

    #[test]
    fn rejects_when_nothing_changed() {
        assert!(detect_edits("Regards, Fan", "Regards, Fan").is_empty());
    }

    #[test]
    fn core_strips_surrounding_punctuation_only() {
        assert_eq!(core("\"Fan,\""), "Fan");
        assert_eq!(core("well-known"), "well-known");
        assert_eq!(core("..."), "");
    }

    fn snap(value: &str) -> Snapshot {
        Snapshot {
            id: "app:AXTextArea:Compose".into(),
            value: value.into(),
            secure: false,
        }
    }

    #[test]
    fn start_refuses_a_secure_field() {
        let mut s = snap("Regards, fun");
        s.secure = true;
        assert!(Watch::start(&s, "Regards, fun").is_err());
    }

    #[test]
    fn start_refuses_text_it_cannot_find() {
        assert!(Watch::start(&snap("something else"), "Regards, fun").is_err());
    }

    #[test]
    fn start_takes_the_last_occurrence_of_a_repeated_span() {
        let mut w = Watch::start(&snap("go on. go on"), "go on").unwrap();
        assert_eq!(w.step(&snap("go on. go One")), Step::Continue);
        match w.step(&snap("go on. go One")) {
            Step::Learned(v) => assert_eq!(v[0].to, "One"),
            other => panic!("expected Learned, got {other:?}"),
        }
    }

    #[test]
    fn start_finds_a_span_the_app_rewrote_with_smart_quotes() {
        let w = Watch::start(&snap("she said \u{201C}hi\u{201D} to me"), "she said \"hi\" to me");
        assert!(w.is_ok());
    }

    #[test]
    fn start_finds_a_span_pasted_with_different_whitespace() {
        // Voice-command dictations paste segments with keystrokes between
        // them, so the field holds a newline where the joined text has a
        // space.
        let w = Watch::start(&snap("first line\nsecond line"), "first line second line");
        assert!(w.is_ok());
    }

    #[test]
    fn learns_after_two_identical_polls() {
        let mut w = Watch::start(&snap("Hi. Regards, fun"), "Regards, fun").unwrap();
        assert_eq!(w.step(&snap("Hi. Regards, F")), Step::Continue);
        assert_eq!(w.step(&snap("Hi. Regards, Fan")), Step::Continue);
        match w.step(&snap("Hi. Regards, Fan")) {
            Step::Learned(v) => assert_eq!(v[0].to, "Fan"),
            other => panic!("expected Learned, got {other:?}"),
        }
    }

    #[test]
    fn a_later_settled_state_supersedes_an_earlier_commit() {
        let mut w = Watch::start(&snap("Regards, fun"), "Regards, fun").unwrap();
        assert_eq!(w.step(&snap("Regards, Fa")), Step::Continue);
        match w.step(&snap("Regards, Fa")) {
            Step::Learned(v) => assert_eq!(v[0].to, "Fa"),
            other => panic!("expected Learned, got {other:?}"),
        }
        assert_eq!(w.step(&snap("Regards, Fan")), Step::Continue);
        match w.step(&snap("Regards, Fan")) {
            Step::Learned(v) => assert_eq!(v[0].to, "Fan"),
            other => panic!("expected Learned, got {other:?}"),
        }
    }

    #[test]
    fn a_revert_after_a_commit_retracts_it() {
        let mut w = Watch::start(&snap("Regards, fun"), "Regards, fun").unwrap();
        assert_eq!(w.step(&snap("Regards, Fan")), Step::Continue);
        assert!(matches!(w.step(&snap("Regards, Fan")), Step::Learned(_)));
        assert_eq!(w.step(&snap("Regards, fun")), Step::Continue);
        assert_eq!(w.step(&snap("Regards, fun")), Step::Learned(Vec::new()));
    }

    #[test]
    fn abandons_when_focus_moves() {
        let mut w = Watch::start(&snap("Regards, fun"), "Regards, fun").unwrap();
        let mut other = snap("Regards, Fan");
        other.id = "other:AXTextField:".into();
        assert_eq!(w.step(&other), Step::Abandon("focus changed"));
    }

    #[test]
    fn abandons_when_the_field_becomes_secure() {
        let mut w = Watch::start(&snap("Regards, fun"), "Regards, fun").unwrap();
        let mut s = snap("Regards, Fan");
        s.secure = true;
        assert_eq!(w.step(&s), Step::Abandon("secure field"));
    }

    #[test]
    fn abandons_when_the_edit_lands_inside_the_near_anchor() {
        // The whole prefix is shorter than the anchor, so editing it loses
        // the span for real.
        let mut w = Watch::start(&snap("Hi. Regards, fun"), "Regards, fun").unwrap();
        assert_eq!(
            w.step(&snap("Hello. Regards, Fan")),
            Step::Abandon("edited outside span")
        );
    }

    #[test]
    fn survives_an_edit_far_before_the_span() {
        let doc = "This opening paragraph has plenty of words in it already. Regards, fun";
        let mut w = Watch::start(&snap(doc), "Regards, fun").unwrap();
        let edited = "That opening paragraph has plenty of words in it already. Regards, Fan";
        assert_eq!(w.step(&snap(edited)), Step::Continue);
        match w.step(&snap(edited)) {
            Step::Learned(v) => assert_eq!(v[0].to, "Fan"),
            other => panic!("expected Learned, got {other:?}"),
        }
    }

    #[test]
    fn tolerates_typing_after_our_text() {
        let mut w = Watch::start(&snap("Regards, fun"), "Regards, fun").unwrap();
        assert_eq!(w.step(&snap("Regards, Fan and team")), Step::Continue);
        match w.step(&snap("Regards, Fan and team")) {
            Step::Learned(v) => {
                assert_eq!(v.len(), 1);
                assert_eq!(v[0].to, "Fan");
            }
            other => panic!("expected Learned, got {other:?}"),
        }
    }

    #[test]
    fn continues_while_nothing_has_changed() {
        let mut w = Watch::start(&snap("Regards, fun"), "Regards, fun").unwrap();
        assert_eq!(w.step(&snap("Regards, fun")), Step::Continue);
    }
}
