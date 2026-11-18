const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "from", "are", "was", "were",
    "have", "has", "had", "not", "but", "you", "your", "can", "will", "into",
    "also", "than", "then", "them", "they", "their", "there", "here", "what",
    "when", "where", "which", "while", "about", "after", "before", "between",
    "each", "other", "some", "such", "only", "same", "very", "just", "more",
    "most", "use", "used", "using", "its", "out", "all", "any", "both", "few",
    "how", "why", "who", "whom", "been", "being", "does", "did", "doing",
    "would", "could", "should", "may", "might", "must", "shall", "over",
    "under", "again", "further", "once", "during", "through", "above", "below",
    "import", "def", "return", "const", "let", "var", "function", "class",
    "true", "false", "null", "none", "void", "int", "str", "self", "public",
    "private", "static", "export", "default", "include", "endif", "ifdef",
];

/// Unicode-aware tokenization for Greek variables, accented words and
/// non-Latin scripts. Mathematical operators are normalized into stable terms.
pub fn extract_raw_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();

    for ch in text.chars() {
        if ch.is_alphanumeric() || (is_combining_mark(ch) && !buf.is_empty()) {
            for lower in ch.to_lowercase() {
                buf.push(lower);
            }
        } else if !buf.is_empty() {
            if buf.chars().count() >= 2 || buf.chars().any(char::is_numeric) {
                out.push(std::mem::take(&mut buf));
            } else {
                buf.clear();
            }
        }
    }
    if !buf.is_empty() && (buf.chars().count() >= 2 || buf.chars().any(char::is_numeric)) {
        out.push(buf);
    }
    out
}

fn is_combining_mark(ch: char) -> bool {
    matches!(ch as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF |
        0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

pub fn tokenize_content(text: &str) -> Vec<String> {
    let mut raw = extract_raw_tokens(text);
    raw.extend(super::math::extract_math_tokens(text));
    let mut seen = std::collections::HashSet::new();
    raw.retain(|token| seen.insert(token.clone()));

    let filtered: Vec<String> = raw.iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .cloned()
        .collect();
    if filtered.is_empty() { raw } else { filtered }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_stopwords_and_short_tokens() {
        let toks = tokenize_content("The quick brown fox and a 123 diagram");
        assert_eq!(toks, vec!["quick", "brown", "fox", "123", "diagram"]);
    }

    #[test]
    fn falls_back_when_all_stopwords() {
        let toks = tokenize_content("the and for");
        assert_eq!(toks, vec!["the", "and", "for"]);
    }

    #[test]
    fn preserves_greek_and_unicode_words() {
        let toks = extract_raw_tokens("Διαφορική εξίσωση: α + β = 12; résumé");
        assert!(toks.contains(&"διαφορική".to_string()));
        assert!(toks.contains(&"εξίσωση".to_string()));
        assert!(toks.contains(&"résumé".to_string()));
        assert!(toks.contains(&"12".to_string()));
    }

    #[test]
    fn includes_math_features_in_regular_search_tokens() {
        let toks = tokenize_content(r"Find \int_0^1 \frac{\alpha}{\sqrt{x}}");
        assert!(toks.contains(&"math:int".to_string()));
        assert!(toks.contains(&"math:frac".to_string()));
        assert!(toks.contains(&"math:alpha".to_string()));
        assert!(toks.contains(&"op:integral".to_string()));
        assert!(toks.contains(&"op:superscript".to_string()));
    }
}
