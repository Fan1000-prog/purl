//! Writing-mode structuring: reshape a cleaned transcript's layout.
//! Rule-based so it behaves identically across Fast / AI / Off cleanup.
//!
//! v0.3 ships one mode: Email — greeting on its own line, body grouped into
//! paragraphs, sign-off (and trailing name) separated at the end.

use crate::settings::StructureMode;

/// Body sentences per paragraph before a blank line is inserted.
const SENTENCES_PER_PARAGRAPH: usize = 3;

const GREETINGS: &[&str] = &[
    "hi", "hey", "hello", "dear", "good morning", "good afternoon", "good evening",
];

const SIGN_OFFS: &[&str] = &[
    "thanks", "thank you", "best regards", "kind regards", "warm regards", "best",
    "regards", "cheers", "sincerely", "talk soon",
];

/// Greetings longer than this many words are treated as body content
/// ("hi everyone I wanted to flag..." said without a pause).
const MAX_GREETING_WORDS: usize = 6;

pub fn apply(text: &str, mode: StructureMode) -> String {
    match mode {
        StructureMode::None => text.to_string(),
        StructureMode::Email => format_email(text),
        StructureMode::Coding => format_coding(text),
    }
}

/// Spoken phrases that start a bullet item in Coding mode. Matched at the
/// beginning of a sentence, case-insensitively.
const BULLET_MARKERS: &[&str] = &["bullet point", "next bullet", "dash"];

/// Coding mode: dictated prompts/technical notes → light markdown.
/// AI cleanup already does the heavy lifting when enabled; this rule-based
/// pass makes Fast/Off cleanup usable too and never harms AI output:
/// - sentences opening with a spoken bullet marker become `- ` list items
/// - an unclosed ``` fence is balanced so pasted markdown never swallows
///   everything after it
fn format_coding(text: &str) -> String {
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return text.trim().to_string();
    }

    let mut lines: Vec<String> = Vec::with_capacity(sentences.len());
    for s in &sentences {
        if let Some(item) = strip_bullet_marker(s) {
            lines.push(format!("- {}", capitalize_first(&item)));
        } else if let Some(last) = lines.last_mut() {
            if !last.starts_with("- ") {
                // Plain prose sentences stay grouped on one line, like input.
                last.push(' ');
                last.push_str(s);
                continue;
            } else {
                lines.push(s.clone());
            }
        } else {
            lines.push(s.clone());
        }
    }

    let mut out = lines.join("\n");
    if out.matches("```").count() % 2 == 1 {
        out.push_str("\n```");
    }
    out
}

/// "Bullet point deploy the API." → Some("deploy the API."); None when the
/// sentence doesn't open with a marker (or is nothing but the marker).
fn strip_bullet_marker(sentence: &str) -> Option<String> {
    let lower = sentence.to_lowercase();
    for marker in BULLET_MARKERS {
        if starts_with_phrase(&lower, &[marker]) {
            // Cut at the marker's word count to keep original casing.
            let n = marker.split(' ').count();
            let rest = sentence.split_whitespace().skip(n).collect::<Vec<_>>().join(" ");
            let bare = rest.trim_start_matches([',', ':']).trim();
            if !bare.is_empty() {
                return Some(bare.to_string());
            }
        }
    }
    None
}

/// Split into sentences at `.` `!` `?`, keeping the punctuation. Existing
/// newlines are treated as sentence breaks too.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    for line in text.split('\n') {
        let mut start = 0;
        for (i, c) in line.char_indices() {
            if ".!?".contains(c) {
                let s = line[start..=i].trim();
                if !s.is_empty() {
                    sentences.push(s.to_string());
                }
                start = i + 1;
            }
        }
        let tail = line[start..].trim();
        if !tail.is_empty() {
            sentences.push(tail.to_string());
        }
    }
    sentences
}

fn starts_with_phrase(sentence: &str, phrases: &[&str]) -> bool {
    let lower = sentence.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .collect();
    phrases.iter().any(|p| {
        let pw: Vec<&str> = p.split(' ').collect();
        words.len() >= pw.len() && words[..pw.len()] == pw[..]
    })
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Words that follow a greeting comma but are never a name, so "Hi, I hope
/// you're well" keeps the comma that is doing real work in it.
const NOT_A_NAME: &[&str] = &["i", "we", "you", "he", "she", "it", "they", "there"];

/// True when the whole phrase is a greeting and nothing else — "hi",
/// "good morning" — as opposed to merely starting with one.
fn is_bare_greeting(s: &str) -> bool {
    let lower = s.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .collect();
    GREETINGS.iter().any(|p| p.split(' ').collect::<Vec<_>>() == words)
}

/// Whisper writes the vocative comma — "Hi, Maria" — because that is what the
/// grammar books say. Nobody types it in an email, and on its own line it
/// reads as a pause where the greeting wants none. Drop that one comma.
///
/// Deliberately narrow: it fires only when a bare greeting is followed by a
/// comma and one or two capitalised name-like words. "Hi, I reviewed the
/// proposal" and "Hi, thanks for sending that" keep their comma, because
/// there it separates real clauses rather than a name.
fn strip_vocative_comma(sentence: &str) -> String {
    let Some((head, rest)) = sentence.split_once(',') else {
        return sentence.to_string();
    };
    let (head, rest) = (head.trim(), rest.trim_start());
    if !is_bare_greeting(head) || rest.is_empty() {
        return sentence.to_string();
    }

    // Only the clause up to the next comma is a candidate name; anything
    // longer is the body of a one-breath sentence.
    let name = rest.split(',').next().unwrap_or(rest).trim();
    let name = name.trim_end_matches(['.', '!', '?']);
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.is_empty() || words.len() > 2 {
        return sentence.to_string();
    }
    let first = words[0].split('\'').next().unwrap_or(words[0]).to_lowercase();
    if NOT_A_NAME.contains(&first.as_str()) {
        return sentence.to_string();
    }
    if !words.iter().all(|w| w.chars().next().is_some_and(char::is_uppercase)) {
        return sentence.to_string();
    }

    format!("{head} {rest}")
}

/// One-breath dictation arrives as a single comma-run sentence
/// ("Hi Sarah, I reviewed..., thanks."). Split a greeting off the front:
/// "Hi Sarah, I reviewed..." → ("Hi Sarah,", "I reviewed...").
fn split_comma_greeting(sentence: &str) -> Option<(String, String)> {
    let (head, rest) = sentence.split_once(',')?;
    let (head, rest) = (head.trim(), rest.trim());
    if rest.is_empty()
        || !starts_with_phrase(head, GREETINGS)
        || word_count(head) > MAX_GREETING_WORDS
    {
        return None;
    }
    Some((format!("{head},"), capitalize_first(rest)))
}

/// Split a sign-off (and optional trailing name) off the back of a comma-run
/// sentence: "...let me know, thanks, Fan." → ("...let me know.", "Thanks,", "Fan.").
fn split_comma_sign_off(sentence: &str) -> Option<(String, String, Option<String>)> {
    let clauses: Vec<&str> = sentence.split(',').map(str::trim).collect();
    if clauses.len() < 2 {
        return None;
    }
    let bare = |s: &str| s.trim_end_matches(['.', '!', '?']).to_string();

    let last = clauses[clauses.len() - 1];
    let (body_end, sign_off, name) =
        if starts_with_phrase(last, SIGN_OFFS) && word_count(last) <= 4 {
            (clauses.len() - 1, capitalize_first(last), None)
        } else if clauses.len() >= 3 && word_count(last) <= 2 {
            let prev = clauses[clauses.len() - 2];
            if starts_with_phrase(prev, SIGN_OFFS) && word_count(prev) <= 4 {
                (
                    clauses.len() - 2,
                    format!("{},", capitalize_first(&bare(prev))),
                    Some(last.to_string()),
                )
            } else {
                return None;
            }
        } else {
            return None;
        };

    let mut body = clauses[..body_end].join(", ");
    if !body.ends_with(['.', '!', '?']) {
        body.push('.');
    }
    Some((body, sign_off, name))
}

fn format_email(text: &str) -> String {
    let mut sentences = split_sentences(text);
    if sentences.is_empty() {
        return text.trim().to_string();
    }
    // Before anything else, so both greeting paths below see "Hi Maria"
    // rather than "Hi, Maria" — otherwise the comma split strands the name
    // at the head of the body.
    sentences[0] = strip_vocative_comma(&sentences[0]);

    let greeting = if starts_with_phrase(&sentences[0], GREETINGS)
        && word_count(&sentences[0]) <= MAX_GREETING_WORDS
    {
        Some(sentences.remove(0))
    } else if let Some((g, rest)) = split_comma_greeting(&sentences[0]) {
        sentences[0] = rest;
        Some(g)
    } else {
        None
    };

    // Sign-off: last sentence, or second-to-last when the final sentence is
    // a bare 1–2 word name ("Thanks. Fan.").
    let mut name: Option<String> = None;
    let mut sign_off: Option<String> = None;
    if let Some(last) = sentences.last() {
        if starts_with_phrase(last, SIGN_OFFS) && word_count(last) <= 4 {
            sign_off = sentences.pop();
        } else if sentences.len() >= 2 && word_count(last) <= 2 {
            let prev = &sentences[sentences.len() - 2];
            if starts_with_phrase(prev, SIGN_OFFS) && word_count(prev) <= 4 {
                name = sentences.pop();
                sign_off = sentences.pop();
            }
        }
    }
    if sign_off.is_none() {
        if let Some(last) = sentences.last_mut() {
            if let Some((body_part, s, n)) = split_comma_sign_off(last) {
                *last = body_part;
                sign_off = Some(s);
                name = n;
            }
        }
    }

    let body = sentences
        .chunks(SENTENCES_PER_PARAGRAPH)
        .map(|para| para.join(" "))
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut parts: Vec<String> = Vec::new();
    if let Some(g) = greeting {
        parts.push(g);
    }
    if !body.is_empty() {
        parts.push(body);
    }
    match (sign_off, name) {
        (Some(s), Some(n)) => parts.push(format!("{s}\n{n}")),
        (Some(s), None) => parts.push(s),
        _ => {}
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_mode_is_passthrough() {
        assert_eq!(apply("Hi team. All good.", StructureMode::None), "Hi team. All good.");
    }

    #[test]
    fn full_email_shape() {
        let input = "Hi Sarah. I reviewed the proposal this morning. The pricing section \
                     looks solid. I think we should move the timeline up a week. Let me know \
                     if that works for your team. Thanks. Fan.";
        let out = apply(input, StructureMode::Email);
        assert_eq!(
            out,
            "Hi Sarah.\n\n\
             I reviewed the proposal this morning. The pricing section looks solid. \
             I think we should move the timeline up a week.\n\n\
             Let me know if that works for your team.\n\n\
             Thanks.\nFan."
        );
    }

    #[test]
    fn sign_off_without_name() {
        let out = apply("Hello team. Quick update on the launch. Best regards.", StructureMode::Email);
        assert_eq!(out, "Hello team.\n\nQuick update on the launch.\n\nBest regards.");
    }

    #[test]
    fn no_greeting_no_sign_off_still_paragraphs() {
        let input = "First point here. Second point here. Third point here. Fourth point here.";
        let out = apply(input, StructureMode::Email);
        assert_eq!(
            out,
            "First point here. Second point here. Third point here.\n\nFourth point here."
        );
    }

    #[test]
    fn drops_the_vocative_comma_in_a_greeting() {
        // Whisper writes "Hi, Maria"; an email greeting wants "Hi Maria".
        let out = apply("Hi, Maria. The report is ready.", StructureMode::Email);
        assert_eq!(out, "Hi Maria.\n\nThe report is ready.");
    }

    #[test]
    fn drops_the_vocative_comma_in_a_one_breath_greeting() {
        // Without the strip, the comma split makes the greeting "Hi," and
        // strands "Maria" at the front of the body.
        let out = apply(
            "Hi, Maria, I reviewed the proposal this morning.",
            StructureMode::Email,
        );
        assert_eq!(out, "Hi Maria,\n\nI reviewed the proposal this morning.");
    }

    #[test]
    fn keeps_a_two_word_name_together() {
        let out = apply("Hi, Maria Lopez. The report is ready.", StructureMode::Email);
        assert_eq!(out, "Hi Maria Lopez.\n\nThe report is ready.");
    }

    #[test]
    fn keeps_the_comma_when_what_follows_is_not_a_name() {
        // "Hi, I reviewed…" — the comma separates clauses, not a name.
        let out = apply("Hi, I reviewed the proposal this morning.", StructureMode::Email);
        assert!(
            out.starts_with("Hi,"),
            "expected the clause comma to survive, got {out:?}"
        );
    }

    #[test]
    fn keeps_the_comma_before_a_lowercase_word() {
        let out = apply("Hi, thanks for sending that over.", StructureMode::Email);
        assert!(
            out.starts_with("Hi,"),
            "expected the clause comma to survive, got {out:?}"
        );
    }

    #[test]
    fn strips_the_comma_after_a_two_word_greeting() {
        let out = apply("Good morning, Maria. The report is ready.", StructureMode::Email);
        assert_eq!(out, "Good morning Maria.\n\nThe report is ready.");
    }

    #[test]
    fn leaves_non_greeting_commas_alone() {
        let out = apply("Maria, the report is ready.", StructureMode::Email);
        assert!(out.starts_with("Maria,"), "got {out:?}");
    }

    #[test]
    fn long_hi_sentence_is_body_not_greeting() {
        let input = "Hi everyone I wanted to flag the migration slipping to Friday.";
        let out = apply(input, StructureMode::Email);
        assert!(!out.starts_with("Hi everyone I wanted to flag the migration slipping to Friday.\n\n\n"));
        assert_eq!(out, input); // single body sentence, no splits
    }

    #[test]
    fn thanks_mid_body_is_not_a_sign_off() {
        let input = "Hi Joe. Thanks for sending the report so quickly over yesterday. I'll read it tonight.";
        let out = apply(input, StructureMode::Email);
        // "Thanks for sending..." is > 4 words → body, not sign-off.
        assert_eq!(
            out,
            "Hi Joe.\n\nThanks for sending the report so quickly over yesterday. I'll read it tonight."
        );
    }

    #[test]
    fn empty_input() {
        assert_eq!(apply("", StructureMode::Email), "");
    }

    // One-breath dictation: Whisper punctuates with commas only, so the whole
    // email arrives as a single sentence. Greeting and sign-off must still be
    // extracted at comma boundaries.
    #[test]
    fn one_breath_comma_only_transcript() {
        let input = "Hi Sarah, I reviewed the proposal this morning, the pricing section \
                     looks solid, I think we should move the timeline up a week, let me \
                     know if that works, thanks.";
        let out = apply(input, StructureMode::Email);
        assert_eq!(
            out,
            "Hi Sarah,\n\n\
             I reviewed the proposal this morning, the pricing section looks solid, \
             I think we should move the timeline up a week, let me know if that works.\n\n\
             Thanks."
        );
    }

    #[test]
    fn one_breath_sign_off_with_name() {
        let input = "Hi Sarah, I reviewed the proposal, let me know if that works, thanks, Fan.";
        let out = apply(input, StructureMode::Email);
        assert_eq!(
            out,
            "Hi Sarah,\n\nI reviewed the proposal, let me know if that works.\n\nThanks,\nFan."
        );
    }

    #[test]
    fn comma_clause_not_a_sign_off_when_long() {
        let input = "Hi Joe, the report was great, thanks so much for sending it over early.";
        let out = apply(input, StructureMode::Email);
        assert_eq!(
            out,
            "Hi Joe,\n\nThe report was great, thanks so much for sending it over early."
        );
    }

    #[test]
    fn coding_mode_turns_spoken_bullets_into_list_items() {
        let input = "Here are the requirements. Bullet point the API must return JSON. \
                     Bullet point retries use exponential backoff. That covers it.";
        let out = apply(input, StructureMode::Coding);
        assert_eq!(
            out,
            "Here are the requirements.\n\
             - The API must return JSON.\n\
             - Retries use exponential backoff.\n\
             That covers it."
        );
    }

    #[test]
    fn coding_mode_balances_unclosed_code_fence() {
        let input = "Use this snippet. ``` npm install ";
        let out = apply(input, StructureMode::Coding);
        assert!(out.ends_with("\n```"), "fence not balanced: {out}");
        assert_eq!(out.matches("```").count() % 2, 0);
    }

    #[test]
    fn coding_mode_passthrough_without_markers() {
        let input = "Write a system prompt for the support agent. Keep it short.";
        assert_eq!(
            apply(input, StructureMode::Coding),
            "Write a system prompt for the support agent. Keep it short."
        );
    }

    #[test]
    fn coding_mode_empty_input() {
        assert_eq!(apply("", StructureMode::Coding), "");
    }

    #[test]
    fn coding_mode_bare_marker_is_kept_as_text() {
        // "Bullet point." with nothing after it isn't a list item.
        assert_eq!(apply("Bullet point.", StructureMode::Coding), "Bullet point.");
    }

    #[test]
    fn no_greeting_word_means_no_comma_greeting_split() {
        let input = "Okay team, here is the update on the launch, we are on track.";
        let out = apply(input, StructureMode::Email);
        assert_eq!(out, input);
    }
}

#[cfg(test)]
mod pipeline_check {
    use super::*;
    use crate::voice_commands;

    // End-to-end for the reported bug: real one-breath transcript from
    // history.jsonl (ts 1784128125) through structure + command parsing.
    #[test]
    fn one_breath_transcript_survives_command_parsing() {
        let raw = "Hi Sarah, I reviewed the proposal this morning, the pricing section \
                   looks solid, I think we should move the timeline up a week, let me \
                   know if that works, thanks.";
        let structured = apply(raw, StructureMode::Email);
        let segments = voice_commands::parse(&structured);
        let final_text = voice_commands::text_only(&segments);
        assert_eq!(
            final_text,
            "Hi Sarah,\n\n\
             I reviewed the proposal this morning, the pricing section looks solid, \
             I think we should move the timeline up a week, let me know if that works.\n\n\
             Thanks."
        );
    }
}
