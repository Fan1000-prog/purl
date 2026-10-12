//! Voice command detection: spoken commands like "go down" become cursor
//! keystrokes instead of literal text.
//!
//! A command only fires when it is a standalone sentence or comma clause —
//! "Go down." and "Hi Nick, go down." move the cursor, but "the numbers go
//! down." pastes literally. Exact-match on the whole clause is the guard
//! against destroying real dictation with false positives.

/// A key the inject layer can synthesize. Each platform maps it to its own
/// key code (see `platform`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Return,
    Tab,
}

/// One piece of an utterance, executed in order at inject time.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Text(String),
    Key(Key),
}

/// Lowercased, punctuation-free phrase → action. Newline phrases map to
/// literal text so they work even in clipboard-only mode.
fn lookup(phrase: &str) -> Option<Segment> {
    let key = match phrase {
        "go down" | "move down" | "cursor down" => Key::Down,
        "go up" | "move up" | "cursor up" => Key::Up,
        "go left" | "move left" | "cursor left" => Key::Left,
        "go right" | "move right" | "cursor right" => Key::Right,
        "press enter" => Key::Return,
        "press tab" => Key::Tab,
        "new line" | "newline" | "next line" => return Some(Segment::Text("\n".into())),
        "new paragraph" => return Some(Segment::Text("\n\n".into())),
        _ => return None,
    };
    Some(Segment::Key(key))
}

/// Normalize a sentence chunk for command lookup: lowercase, strip
/// punctuation, collapse whitespace.
fn normalize(chunk: &str) -> String {
    chunk
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split `text` into ordered segments. Sentence chunks (split at `.` `!` `?`
/// and newlines) that exactly match a command phrase become actions; all
/// other chunks are re-joined as literal text, original punctuation kept.
pub fn parse(text: &str) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut pending = String::new();

    let flush = |pending: &mut String, segments: &mut Vec<Segment>| {
        let t = pending.trim();
        if !t.is_empty() {
            segments.push(Segment::Text(t.to_string()));
        }
        pending.clear();
    };

    for line in text.split('\n') {
        let mut chunk_start = 0;
        let bytes: Vec<(usize, char)> = line.char_indices().collect();
        let mut i = 0;
        while i <= bytes.len() {
            let at_end = i == bytes.len();
            // Sentence punctuation always breaks. A comma breaks only when
            // followed by whitespace (or end of line) so "1,000" stays whole —
            // one-breath dictation attaches commands as comma clauses
            // ("Hi Nick, go down.") rather than standalone sentences.
            let is_break = !at_end
                && (".!?".contains(bytes[i].1)
                    || (bytes[i].1 == ','
                        && bytes
                            .get(i + 1)
                            .is_none_or(|(_, next)| next.is_whitespace())));
            if at_end || is_break {
                let end = if at_end { line.len() } else { bytes[i].0 + 1 };
                let chunk = &line[chunk_start..end];
                match lookup(&normalize(chunk)) {
                    Some(Segment::Key(k)) => {
                        flush(&mut pending, &mut segments);
                        segments.push(Segment::Key(k));
                    }
                    Some(Segment::Text(t)) => {
                        // Newline token: append to the running text so it
                        // pastes as part of the surrounding content.
                        pending = pending.trim_end().to_string();
                        pending.push_str(&t);
                    }
                    None => {
                        let chunk = chunk.trim();
                        if !chunk.is_empty() {
                            if !pending.is_empty() && !pending.ends_with('\n') {
                                pending.push(' ');
                            }
                            pending.push_str(chunk);
                        }
                    }
                }
                chunk_start = end;
            }
            i += 1;
        }
        if !pending.is_empty() {
            // Preserve the newline that separated input lines — including
            // blank lines, so "\n\n" paragraph breaks survive.
            pending.push('\n');
        }
    }
    let trimmed = pending.trim().to_string();
    if !trimmed.is_empty() {
        segments.push(Segment::Text(trimmed));
    }
    segments
}

/// The text-only rendering of parsed segments — used when keystrokes can't
/// be sent (clipboard-only mode / no Accessibility permission).
pub fn text_only(segments: &[Segment]) -> String {
    segments
        .iter()
        .filter_map(|s| match s {
            Segment::Text(t) => Some(t.as_str()),
            Segment::Key(_) => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// True when any segment needs a synthesized keystroke.
pub fn has_keys(segments: &[Segment]) -> bool {
    segments.iter().any(|s| matches!(s, Segment::Key(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_command_becomes_key() {
        assert_eq!(parse("Go down."), vec![Segment::Key(Key::Down)]);
        assert_eq!(parse("go down"), vec![Segment::Key(Key::Down)]);
    }

    #[test]
    fn embedded_phrase_stays_text() {
        assert_eq!(
            parse("The numbers go down every week."),
            vec![Segment::Text("The numbers go down every week.".into())]
        );
    }

    #[test]
    fn mixed_text_and_commands_keep_order() {
        assert_eq!(
            parse("Thanks for the update. Go down. I'll review tomorrow."),
            vec![
                Segment::Text("Thanks for the update.".into()),
                Segment::Key(Key::Down),
                Segment::Text("I'll review tomorrow.".into()),
            ]
        );
    }

    #[test]
    fn newline_tokens_become_literal_newlines() {
        assert_eq!(
            parse("Hi team. New line. Here's the plan."),
            vec![Segment::Text("Hi team.\nHere's the plan.".into())]
        );
        assert_eq!(
            parse("Hi team. New paragraph. Here's the plan."),
            vec![Segment::Text("Hi team.\n\nHere's the plan.".into())]
        );
    }

    #[test]
    fn multiple_commands_in_a_row() {
        assert_eq!(
            parse("Go down. Go down."),
            vec![Segment::Key(Key::Down), Segment::Key(Key::Down)]
        );
    }

    #[test]
    fn press_enter_and_tab() {
        assert_eq!(
            parse("Press enter. Press tab."),
            vec![Segment::Key(Key::Return), Segment::Key(Key::Tab)]
        );
    }

    #[test]
    fn text_only_drops_keys() {
        let segs = parse("Hello. Go down. World.");
        assert_eq!(text_only(&segs), "Hello. World.");
        assert!(has_keys(&segs));
    }

    #[test]
    fn plain_text_has_no_keys() {
        let segs = parse("Just a normal sentence.");
        assert!(!has_keys(&segs));
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(parse("").is_empty());
        assert!(parse("   ").is_empty());
    }

    #[test]
    fn preserves_existing_newlines_in_text() {
        assert_eq!(
            parse("First line.\nSecond line."),
            vec![Segment::Text("First line.\nSecond line.".into())]
        );
    }

    // One-breath dictation: speech-to-text can attach commands as comma clauses
    // ("Hi Nick, go down.") instead of standalone sentences.
    #[test]
    fn comma_separated_command_becomes_key() {
        assert_eq!(
            parse("High-Nick, Go Down."),
            vec![Segment::Text("High-Nick,".into()), Segment::Key(Key::Down)]
        );
    }

    #[test]
    fn comma_command_mid_sentence() {
        assert_eq!(
            parse("Thank you for reaching out, go down, I'll flag this for review."),
            vec![
                Segment::Text("Thank you for reaching out,".into()),
                Segment::Key(Key::Down),
                Segment::Text("I'll flag this for review.".into()),
            ]
        );
    }

    #[test]
    fn comma_clause_with_extra_words_stays_text() {
        assert_eq!(
            parse("If conversions go down, alert me."),
            vec![Segment::Text("If conversions go down, alert me.".into())]
        );
    }

    #[test]
    fn comma_inside_number_is_not_a_boundary() {
        assert_eq!(
            parse("The total is 1,000 dollars."),
            vec![Segment::Text("The total is 1,000 dollars.".into())]
        );
    }

    #[test]
    fn comma_separated_newline_token() {
        assert_eq!(
            parse("Hi team, new line, here's the plan."),
            vec![Segment::Text("Hi team,\nhere's the plan.".into())]
        );
    }

    #[test]
    fn preserves_blank_line_paragraph_breaks() {
        // Email structure mode emits "\n\n" — it must survive command parsing.
        assert_eq!(
            parse("Hi Sarah.\n\nFirst paragraph.\n\nThanks.\nFan."),
            vec![Segment::Text(
                "Hi Sarah.\n\nFirst paragraph.\n\nThanks.\nFan.".into()
            )]
        );
    }
}
