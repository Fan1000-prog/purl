//! Word error rate: word-level edit distance over the reference length,
//! after lowercasing and dropping punctuation.

pub fn wer(reference: &str, hypothesis: &str) -> f64 {
    let (r, h) = (words(reference), words(hypothesis));
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    edit_distance(&r, &h) as f64 / r.len() as f64
}

/// Lowercase words with punctuation removed. Apostrophes inside a word stay
/// ("don't"), so contractions still count as one word.
pub fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .collect::<String>()
                .trim_matches('\'')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Levenshtein distance over words, one row at a time.
pub fn edit_distance(a: &[String], b: &[String]) -> usize {
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
    fn wer_keeps_contractions_whole() {
        assert_eq!(wer("Don't stop.", "don't stop"), 0.0);
        assert!((wer("don't stop", "dont stop") - 0.5).abs() < 1e-9);
    }

    #[test]
    fn wer_counts_insertions() {
        assert!((wer("a b", "a x b y") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn wer_empty_reference() {
        assert_eq!(wer("", ""), 0.0);
        assert_eq!(wer("", "noise"), 1.0);
    }
}
