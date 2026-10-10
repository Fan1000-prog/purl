//! Personal dictionary: words Purl should spell your way. Feeds fast
//! cleanup's casing fixes and the cleanup LLM's system prompt.
//!
//! Format: one term per line. Optional `term = expansion` for acronyms.
//! Lines starting with `#` and blank lines are ignored.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub term: String,
    pub expansion: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    pub entries: Vec<Entry>,
}

impl Dictionary {
    pub fn parse(text: &str) -> Self {
        let entries = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|line| match line.split_once('=') {
                Some((term, expansion)) => Entry {
                    term: term.trim().to_string(),
                    expansion: Some(expansion.trim().to_string()),
                },
                None => Entry {
                    term: line.to_string(),
                    expansion: None,
                },
            })
            .filter(|e| !e.term.is_empty())
            .collect();
        Dictionary { entries }
    }

    /// Load the user's dictionary. A missing or unreadable file is an empty
    /// dictionary: Purl ships no words of its own.
    pub fn load(user_path: &Path) -> Self {
        std::fs::read_to_string(user_path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Newline list with expansions, for the LLM cleanup system prompt.
    pub fn llm_terms(&self) -> String {
        self.entries
            .iter()
            .map(|e| match &e.expansion {
                Some(x) => format!("- {} ({})", e.term, x),
                None => format!("- {}", e.term),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A copy with extra terms **prepended**, skipping ones already present.
    /// Used for learned corrections, which bias the local LLM but are
    /// deliberately kept out of cloud prompts.
    ///
    /// Prepended so learned terms lead the list. Both the prewarm and the
    /// real cleanup call build the dictionary the same way, so the Ollama
    /// KV-cache prefix stays identical.
    pub fn with_extra_terms(&self, terms: &[String]) -> Dictionary {
        let mut entries: Vec<Entry> = Vec::with_capacity(self.entries.len() + terms.len());
        for t in terms {
            let t = t.trim();
            if t.is_empty()
                || self.entries.iter().any(|e| e.term.eq_ignore_ascii_case(t))
                || entries.iter().any(|e| e.term.eq_ignore_ascii_case(t))
            {
                continue;
            }
            entries.push(Entry { term: t.to_string(), expansion: None });
        }
        entries.extend(self.entries.iter().cloned());
        Dictionary { entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skips_comments_and_blanks() {
        let d = Dictionary::parse("# comment\n\nSKU = stock keeping unit\nQBR\n  \n");
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].term, "SKU");
        assert_eq!(d.entries[0].expansion.as_deref(), Some("stock keeping unit"));
        assert_eq!(d.entries[1].term, "QBR");
        assert_eq!(d.entries[1].expansion, None);
    }

    #[test]
    fn llm_terms_includes_expansions() {
        let d = Dictionary::parse("SKU = stock keeping unit\nQBR");
        assert_eq!(d.llm_terms(), "- SKU (stock keeping unit)\n- QBR");
    }

    #[test]
    fn missing_file_is_an_empty_dictionary() {
        let d = Dictionary::load(std::path::Path::new("/nonexistent/purl-dict.txt"));
        assert!(d.entries.is_empty());
    }

    #[test]
    fn with_extra_terms_prepends_without_duplicating() {
        let d = Dictionary::parse("SKU = stock keeping unit");
        let d2 = d.with_extra_terms(&["Fan".to_string(), "SKU".to_string(), "Fan".to_string()]);
        assert_eq!(d2.entries.len(), 2);
        assert_eq!(d2.entries[0].term, "Fan");
        assert_eq!(d2.entries[1].term, "SKU");
        // The original is untouched — the cloud path keeps using it.
        assert_eq!(d.entries.len(), 1);
    }
}
