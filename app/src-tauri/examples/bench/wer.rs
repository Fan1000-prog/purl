//! Word error rate: word-level edit distance over the reference length,
//! after lowercasing and dropping punctuation.

#[cfg(test)]
pub fn wer(reference: &str, hypothesis: &str) -> f64 {
    corpus_wer([(reference, hypothesis)])
}

/// Total word edits over total reference words, so long clips weigh more
/// than short ones. An empty reference scores 1.0 if anything was heard.
pub fn corpus_wer<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> f64 {
    let (mut edits, mut words) = (0, 0);
    for (reference, hypothesis) in pairs {
        let (r, h) = (words_of(reference), words_of(hypothesis));
        edits += edit_distance(&r, &h);
        words += r.len();
    }
    match (edits, words) {
        (0, _) => 0.0,
        (_, 0) => 1.0,
        _ => edits as f64 / words as f64,
    }
}

/// Lowercase words, split at anything that isn't a letter, digit or
/// apostrophe ("flour-fattened" is two words). Apostrophes inside a word
/// stay, so "don't" is one word; a curly one counts as straight.
fn words_of(text: &str) -> Vec<String> {
    text.chars()
        .map(|c| match c {
            '\u{2019}' => '\'',
            c if c.is_alphanumeric() || c == '\'' => c,
            _ => ' ',
        })
        .collect::<String>()
        .split_whitespace()
        .map(|w| w.trim_matches('\'').to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Levenshtein distance over words, one row at a time.
fn edit_distance(a: &[String], b: &[String]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, wa) in a.iter().enumerate() {
        let mut row = vec![i + 1; b.len() + 1];
        for (j, wb) in b.iter().enumerate() {
            let swap = prev[j] + usize::from(wa != wb);
            row[j + 1] = swap.min(prev[j + 1] + 1).min(row[j] + 1);
        }
        prev = row;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wer_basic() {
        assert!((wer("the cat sat", "the cat sat") - 0.0).abs() < 1e-9);
        assert!((wer("the cat sat", "the bat sat") - 1.0 / 3.0).abs() < 1e-9);
        assert!((wer("a b c d", "a c d") - 0.25).abs() < 1e-9);
    }

    #[test]
    fn wer_normalizes_case_and_punct() {
        assert_eq!(wer("Hello, World!", "hello world"), 0.0);
    }

    #[test]
    fn wer_splits_hyphenated_words() {
        assert_eq!(wer("flour fattened sauce", "flour-fattened sauce"), 0.0);
    }

    #[test]
    fn wer_keeps_contractions_whole() {
        assert_eq!(wer("Don't stop.", "don't stop"), 0.0);
        assert!((wer("don't stop", "dont stop") - 0.5).abs() < 1e-9);
        assert_eq!(wer("don't", "don\u{2019}t"), 0.0);
    }

    #[test]
    fn wer_counts_insertions() {
        assert!((wer("a b", "a x b y") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn corpus_wer_weighs_by_length() {
        let pairs = [("a b c d", "a b c d"), ("x", "y")];
        assert!((corpus_wer(pairs) - 0.2).abs() < 1e-9);
    }

    #[test]
    fn wer_empty_reference() {
        assert_eq!(wer("", ""), 0.0);
        assert_eq!(wer("", "noise"), 1.0);
    }
}
