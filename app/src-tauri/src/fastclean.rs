//! Instant rule-based transcript cleanup — the default alternative to the
//! LLM pass, which costs seconds per dictation on CPU-only machines.
//!
//! Deliberately conservative: strips only unambiguous filler words and fixes
//! dictionary term casing. The speech engine already provides punctuation and
//! casing; false-start removal and rephrasing are left to Smart cleanup.

use crate::dictionary::Dictionary;

/// Words that are filler in any context. "like" / "you know" are excluded on
/// purpose — they're often real content and rules can't tell the difference.
const FILLERS: &[&str] = &["um", "uh", "uhm", "erm", "hmm", "mm-hmm", "mmhmm"];

pub fn clean(raw: &str, dictionary: &Dictionary) -> String {
    let no_fillers = strip_fillers(raw);
    let cased = fix_dictionary_casing(&no_fillers, dictionary);
    tidy(&cased)
}

/// Drop filler tokens. A token carries its punctuation ("um," → gone), but a
/// filler's trailing sentence-ender is kept so sentences don't merge.
fn strip_fillers(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        let core: String = token
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect();
        if FILLERS.contains(&core.to_lowercase().as_str()) {
            if let Some(end) = token.chars().last().filter(|c| ".!?".contains(*c)) {
                match out.last_mut() {
                    Some(prev) if !prev.ends_with(['.', '!', '?']) => {
                        // Move the sentence break onto the previous word,
                        // replacing a softer trailing comma if present.
                        if prev.ends_with([',', ';', ':']) {
                            prev.pop();
                        }
                        prev.push(end);
                    }
                    _ => {}
                }
            }
            continue;
        }
        out.push(token.to_string());
    }
    out.join(" ")
}

/// Rewrite dictionary terms to their canonical casing wherever they appear
/// (case-insensitive, word-boundary). Multi-word terms are matched too.
fn fix_dictionary_casing(text: &str, dictionary: &Dictionary) -> String {
    let mut result = text.to_string();
    for entry in &dictionary.entries {
        let term = entry.term.trim();
        if term.is_empty() {
            continue;
        }
        result = replace_word_ci(&result, term);
    }
    result
}

/// Case-insensitively replace whole-word occurrences of `term` with `term`'s
/// own casing.
fn replace_word_ci(text: &str, term: &str) -> String {
    let lower_text = text.to_lowercase();
    let lower_term = term.to_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(pos) = lower_text[cursor..].find(&lower_term) {
        let start = cursor + pos;
        let end = start + term.len();
        let boundary_before = start == 0
            || !text[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
        let boundary_after = end >= text.len()
            || !text[end..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        out.push_str(&text[cursor..start]);
        if boundary_before && boundary_after {
            out.push_str(term);
        } else {
            out.push_str(&text[start..end]);
        }
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Fix the artifacts filler removal can leave behind: doubled punctuation,
/// stray commas, missing capital at a sentence start.
fn tidy(text: &str) -> String {
    let mut s = text.trim().to_string();
    for (from, to) in [
        (",,", ","),
        (", .", "."),
        (",.", "."),
        (" ,", ","),
        (" .", "."),
    ] {
        while s.contains(from) {
            s = s.replace(from, to);
        }
    }
    // Capitalize the first letter and any letter following a sentence end.
    let mut out = String::with_capacity(s.len());
    let mut capitalize = true;
    for c in s.chars() {
        if capitalize && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            capitalize = false;
        } else {
            out.push(c);
            if ".!?".contains(c) {
                capitalize = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dictionary() -> Dictionary {
        Dictionary::parse(
            "API = application programming interface\nQBR = quarterly business review\nAcme Studio",
        )
    }

    #[test]
    fn strips_fillers_and_their_commas() {
        assert_eq!(
            clean("Okay, um, so the report is uh ready.", &dictionary()),
            "Okay, so the report is ready."
        );
    }

    #[test]
    fn keeps_sentence_break_when_filler_ends_sentence() {
        assert_eq!(
            clean("I sent it already, um. Next topic.", &dictionary()),
            "I sent it already. Next topic."
        );
    }

    #[test]
    fn seed_entry_fixes_casing_but_leaves_pearl_alone() {
        let seed = Dictionary::parse(crate::dictionary::SEED);
        assert_eq!(clean("I use purl daily", &seed), "I use Purl daily");
        // Parakeet hears "Purl" as "Pearl", but pearl is a real word.
        assert_eq!(
            clean("Hello, Pearl. A pearl necklace.", &seed),
            "Hello, Pearl. A pearl necklace."
        );
    }

    #[test]
    fn fixes_dictionary_casing_word_boundary() {
        assert_eq!(
            clean("the api notes are in the qbr deck now", &dictionary()),
            "The API notes are in the QBR deck now"
        );
        // "rapid" contains "api" — must not be touched.
        assert_eq!(
            clean("rapid progress on qbr", &dictionary()),
            "Rapid progress on QBR"
        );
    }

    #[test]
    fn fixes_multiword_dictionary_terms() {
        assert_eq!(
            clean("kicked off the acme studio launch", &dictionary()),
            "Kicked off the Acme Studio launch"
        );
    }

    #[test]
    fn capitalizes_sentence_starts() {
        assert_eq!(
            clean("hello there. how are you?", &dictionary()),
            "Hello there. How are you?"
        );
    }

    #[test]
    fn leaves_real_words_alone() {
        // "like" and "you know" are content here — must survive.
        assert_eq!(
            clean("I like this, you know the answer.", &dictionary()),
            "I like this, you know the answer."
        );
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(clean("", &dictionary()), "");
    }
}
