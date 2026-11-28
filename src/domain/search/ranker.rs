use crate::domain::search::tokenizer::extract_raw_tokens;
use std::collections::HashSet;

/// Similarity ratio matching Python's `difflib.SequenceMatcher(None, a, b).ratio()`.
pub fn similarity_ratio(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let total = a_chars.len() + b_chars.len();
    if total == 0 {
        return 1.0;
    }
    if a_chars.is_empty() || b_chars.is_empty() {
        return 0.0;
    }

    let mut dp = vec![0usize; b_chars.len() + 1];
    for &ca in &a_chars {
        let mut prev = 0;
        for (j, &cb) in b_chars.iter().enumerate() {
            let temp = dp[j + 1];
            if ca == cb {
                dp[j + 1] = prev + 1;
            } else if dp[j] > dp[j + 1] {
                dp[j + 1] = dp[j];
            }
            prev = temp;
        }
    }
    let matches = dp[b_chars.len()];
    (2.0 * (matches as f64)) / (total as f64)
}

pub fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Check for contiguous phrase match and extract the highest-density passage snippet.
pub fn inspect_passage_and_phrase(
    title: &str,
    body: &str,
    raw_query_norm: &str,
    q_tokens: &[String],
    existing_snippet: &str,
) -> (bool, String) {
    let body_slice: String = body.chars().take(35_000).collect();
    let hay = format!("{title}\n{body_slice}");
    let collapsed_orig = collapse_whitespace(&hay);
    let norm_hay = collapsed_orig.to_ascii_lowercase();

    let phrase_hit = raw_query_norm.chars().count() >= 6 && norm_hay.contains(raw_query_norm);

    if !existing_snippet.is_empty() && !phrase_hit {
        return (false, existing_snippet.to_string());
    }

    if phrase_hit {
        if let Some(byte_idx) = norm_hay.find(raw_query_norm) {
            let char_idx = norm_hay[..byte_idx].chars().count();
            let q_len = raw_query_norm.chars().count();
            let chars: Vec<char> = collapsed_orig.chars().collect();
            let start = char_idx.saturating_sub(24);
            let end = (char_idx + q_len + 56).min(chars.len());
            let mut snippet: String = chars[start..end].iter().collect();
            snippet = snippet.trim().to_string();
            if start > 0 {
                snippet = format!("…{snippet}");
            }
            if end < chars.len() {
                snippet.push('…');
            }
            return (true, snippet);
        }
    }

    let tok_set: HashSet<&str> = q_tokens.iter().map(|s| s.as_str()).collect();
    let mut best_line = existing_snippet.to_string();
    let mut best_hits = 0usize;

    let body_lines_src: String = body.chars().take(25_000).collect();
    for line in body_lines_src.lines() {
        let clean = collapse_whitespace(line);
        if clean.chars().count() < 4 {
            continue;
        }
        let line_toks = extract_raw_tokens(&clean);
        let hits = line_toks
            .iter()
            .filter(|t| tok_set.contains(t.as_str()))
            .collect::<HashSet<_>>()
            .len();
        if hits > best_hits {
            best_hits = hits;
            best_line = clean.chars().take(120).collect();
        }
    }

    if best_hits == 0 && best_line.is_empty() {
        let short_src: String = body.chars().take(4_000).collect();
        for line in short_src.lines() {
            let clean = collapse_whitespace(line);
            if clean.chars().count() >= 8 {
                best_line = clean.chars().take(120).collect();
                break;
            }
        }
    }

    (phrase_hit, best_line)
}

/// 1-based line of the best match in `body`: the first line containing the whole
/// phrase, otherwise the line with the most distinct query tokens.
pub fn locate_match_line(body: &str, raw_query_norm: &str, q_tokens: &[String]) -> Option<usize> {
    let tok_set: HashSet<&str> = q_tokens.iter().map(|s| s.as_str()).collect();
    let mut best: Option<(usize, usize)> = None; // (hits, line_no)
    for (idx, line) in body.lines().enumerate() {
        let clean = collapse_whitespace(line).to_ascii_lowercase();
        if clean.is_empty() {
            continue;
        }
        if !raw_query_norm.is_empty() && clean.contains(raw_query_norm) {
            return Some(idx + 1);
        }
        let hits = extract_raw_tokens(&clean)
            .iter()
            .filter(|t| tok_set.contains(t.as_str()))
            .collect::<HashSet<_>>()
            .len();
        if hits > best.map(|b| b.0).unwrap_or(0) {
            best = Some((hits, idx + 1));
        }
    }
    best.map(|(_, line)| line)
}

/// Index into `pages` of the page that best matches: pages containing the whole
/// phrase win, then pages with the most distinct query tokens (earlier page on ties).
pub fn best_pdf_page(
    pages: &[(i64, String, String)],
    raw_query_norm: &str,
    q_tokens: &[String],
) -> Option<usize> {
    let tok_set: HashSet<&str> = q_tokens.iter().map(|s| s.as_str()).collect();
    let mut best: Option<(usize, usize)> = None; // (score, index)
    for (i, (_, body, _)) in pages.iter().enumerate() {
        let norm = collapse_whitespace(body).to_ascii_lowercase();
        let hits = extract_raw_tokens(&norm)
            .iter()
            .filter(|t| tok_set.contains(t.as_str()))
            .collect::<HashSet<_>>()
            .len();
        let phrase = !raw_query_norm.is_empty() && norm.contains(raw_query_norm);
        let score = if phrase { 1000 } else { 0 } + hits;
        if score > best.map(|b| b.0).unwrap_or(0) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Score filename/path relevance so `gsearch query` serves both direct file search and content search.
pub fn score_filename_match(path: &str, name: &str, q_lower: &str, q_tokens: &[String]) -> f64 {
    let name_lower = name.to_ascii_lowercase();
    let path_lower = path.to_ascii_lowercase();

    if name_lower == q_lower {
        return 950.0;
    }
    if name_lower.starts_with(q_lower) {
        return 720.0;
    }
    if name_lower.contains(q_lower) {
        return 540.0;
    }
    if !q_tokens.is_empty() && q_tokens.iter().all(|t| name_lower.contains(t)) {
        return 420.0;
    }
    if !q_tokens.is_empty() && q_tokens.iter().all(|t| path_lower.contains(t)) {
        return 260.0;
    }
    0.0
}
