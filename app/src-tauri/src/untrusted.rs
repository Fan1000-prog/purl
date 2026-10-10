//! Text from outside the user's control (another app's selection, words read
//! back from another app's field) on its way into an LLM prompt. The prompt
//! wraps it in tags; nothing in it may open or close one, whatever the
//! spacing, case or bracket lookalike.

/// `<` and characters that render like it.
fn is_open_angle(c: char) -> bool {
    matches!(c, '<' | '\u{FF1C}' | '\u{FE64}' | '\u{2039}' | '\u{3008}')
}

/// `>` and characters that render like it.
fn is_close_angle(c: char) -> bool {
    matches!(c, '>' | '\u{FF1E}' | '\u{FE65}' | '\u{203A}' | '\u{3009}')
}

/// Invisible characters that reorder or break what a reader sees: bidi
/// embeddings/overrides/isolates and the Unicode line/paragraph separators.
fn is_hidden_format(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}')
}

/// Drop every angle bracket (ASCII or lookalike) and hidden format character.
/// For short terms, where a bracket is never legitimate.
pub fn strip_angles_and_hidden(s: &str) -> String {
    s.chars()
        .filter(|&c| !is_open_angle(c) && !is_close_angle(c) && !is_hidden_format(c))
        .collect()
}

/// A tag found in untrusted text: its byte range and whether it closes.
#[derive(Debug, PartialEq)]
pub struct Tag {
    pub start: usize,
    pub end: usize,
    pub closing: bool,
}

/// Every `<\s*/?\s*name\s*>` in `s`, case-insensitive, with any of the
/// bracket lookalikes standing in for `<` and `>`.
pub fn find_tags(s: &str, name: &str) -> Vec<Tag> {
    let mut tags = Vec::new();
    let mut search_from = 0;
    while let Some(off) = s[search_from..].find(is_open_angle) {
        let start = search_from + off;
        let open_len = s[start..].chars().next().map_or(1, char::len_utf8);
        match tag_tail(&s[start + open_len..], name) {
            Some((len, closing)) => {
                let end = start + open_len + len;
                tags.push(Tag {
                    start,
                    end,
                    closing,
                });
                search_from = end;
            }
            None => search_from = start + open_len,
        }
    }
    tags
}

/// After an opening bracket: `\s*/?\s*name\s*>`. Returns the bytes consumed
/// and whether the tag closes.
fn tag_tail(rest: &str, name: &str) -> Option<(usize, bool)> {
    let skip_ws = |s: &str| s.len() - s.trim_start().len();
    let mut i = skip_ws(rest);
    let closing = rest[i..].starts_with('/');
    if closing {
        i += 1;
        i += skip_ws(&rest[i..]);
    }
    let candidate = rest[i..].get(..name.len())?;
    if !candidate.eq_ignore_ascii_case(name) {
        return None;
    }
    i += name.len();
    i += skip_ws(&rest[i..]);
    let c = rest[i..].chars().next()?;
    is_close_angle(c).then_some((i + c.len_utf8(), closing))
}

/// Replace every `name` tag variant with `[name]` / `[/name]` and drop hidden
/// format characters. Everything else, `<` included, is left as written.
pub fn defang_tags(s: &str, name: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for t in find_tags(s, name) {
        out.push_str(&s[last..t.start]);
        out.push_str(&if t.closing {
            format!("[/{name}]")
        } else {
            format!("[{name}]")
        });
        last = t.end;
    }
    out.push_str(&s[last..]);
    out.retain(|c| !is_hidden_format(c));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_spacing_case_and_lookalike_variants() {
        for t in [
            "<text>",
            "< TEXT >",
            "</text>",
            "</ text>",
            "< /text >",
            "</text\n>",
            "\u{FF1C}/text\u{FF1E}",
            "\u{FE64}text\u{FE65}",
            "\u{2039}/Text\u{203A}",
            "\u{3008}text\u{3009}",
        ] {
            assert_eq!(find_tags(t, "text").len(), 1, "{t:?}");
        }
    }

    #[test]
    fn leaves_ordinary_brackets_alone() {
        assert!(find_tags("a < b && Vec<u8> <textarea> <tex>", "text").is_empty());
        assert_eq!(defang_tags("a < b && Vec<u8>", "text"), "a < b && Vec<u8>");
    }

    #[test]
    fn strips_angles_and_hidden_formatting() {
        assert_eq!(
            strip_angles_and_hidden("\u{FF1C}/x\u{FF1E} <y> \u{202E}z\u{2028}"),
            "/x y z"
        );
    }
}
