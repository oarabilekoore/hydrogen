use crate::domain::search::tokenizer::tokenize_content;
use std::collections::HashMap;

pub const MAX_DOC_TERMS: usize = 600;

/// Extract sublinear TF weights for exact unigrams (`w:`), bigrams (`b:`), and visual captions.
pub fn extract_doc_terms(title: &str, body: &str, caption_text: &str) -> Vec<(String, f64)> {
    let title_toks = tokenize_content(title);
    let body_toks = tokenize_content(body);
    let cap_toks = if caption_text.is_empty() {
        Vec::new()
    } else {
        tokenize_content(caption_text)
    };

    let mut tf: HashMap<String, f64> = HashMap::new();

    for tok in &title_toks {
        *tf.entry(format!("w:{tok}")).or_insert(0.0) += 3.5;
    }
    for pair in title_toks.windows(2) {
        *tf.entry(format!("b:{}_{}", pair[0], pair[1])).or_insert(0.0) += 3.0;
    }

    for tok in &cap_toks {
        *tf.entry(format!("w:{tok}")).or_insert(0.0) += 2.2;
    }
    for pair in cap_toks.windows(2) {
        *tf.entry(format!("b:{}_{}", pair[0], pair[1])).or_insert(0.0) += 2.0;
    }

    for tok in &body_toks {
        *tf.entry(format!("w:{tok}")).or_insert(0.0) += 1.0;
    }
    for pair in body_toks.windows(2) {
        *tf.entry(format!("b:{}_{}", pair[0], pair[1])).or_insert(0.0) += 1.6;
    }

    let mut weighted: Vec<(String, f64)> = tf
        .into_iter()
        .filter(|(_, val)| *val > 0.0)
        .map(|(term, val)| {
            let w = ((1.0 + val.ln()) * 10_000.0).round() / 10_000.0;
            (term, w)
        })
        .collect();

    if weighted.len() > MAX_DOC_TERMS {
        weighted.sort_by(|a, b| b.1.total_cmp(&a.1));
        weighted.truncate(MAX_DOC_TERMS);
    }
    weighted
}

pub fn compute_idf(total_docs: usize, df: usize) -> f64 {
    let raw = 1.0 + (((total_docs as f64) + 1.0) / ((df as f64) + 1.0)).ln();
    (raw * 10_000.0).round() / 10_000.0
}

pub fn normalize_doc_weights(items: &[(String, f64)], has_body: bool) -> Vec<(String, f64)> {
    if items.is_empty() {
        return Vec::new();
    }
    let sq_sum: f64 = items.iter().map(|(_, w)| w * w).sum();
    let raw_norm = sq_sum.sqrt();
    let min_norm = if has_body { 2.5 } else { 7.0 };
    let norm = raw_norm.max(min_norm);
    let inv = 1.0 / norm;

    items
        .iter()
        .map(|(t, w)| (t.clone(), ((w * inv) * 100_000.0).round() / 100_000.0))
        .collect()
}
