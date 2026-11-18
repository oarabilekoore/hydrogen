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

pub fn extract_raw_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            buf.push(ch.to_ascii_lowercase());
        } else if !buf.is_empty() {
            if buf.len() >= 2 {
                out.push(std::mem::take(&mut buf));
            } else {
                buf.clear();
            }
        }
    }
    if buf.len() >= 2 {
        out.push(buf);
    }
    out
}

pub fn tokenize_content(text: &str) -> Vec<String> {
    let raw = extract_raw_tokens(text);
    let filtered: Vec<String> = raw
        .iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .cloned()
        .collect();
    if filtered.is_empty() {
        raw
    } else {
        filtered
    }
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
}
