use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::config::AppConfig;
use crate::error::Result;
use crate::ports::repository::IndexRepository;
use clap::Args;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Args, Debug)]
pub struct AnswerArgs {
    /// Question to answer from indexed local documents
    pub query: String,
    /// Maximum number of retrieved passages used as context
    #[arg(long)]
    pub limit: Option<usize>,
    /// Ollama model tag
    #[arg(long)]
    pub model: Option<String>,
}

fn query_variants(query: &str, max_variants: usize) -> Vec<String> {
    const STOP: &[&str] = &[
        "what", "which", "where", "when", "who", "why", "how", "is", "are", "was",
        "were", "the", "a", "an", "in", "on", "of", "for", "to", "and", "or",
        "with", "from", "explain", "describe", "define", "tell", "me", "about",
    ];
    let mut variants = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let cleaned: Vec<String> = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_lowercase)
        .filter(|word| !STOP.contains(&word.as_str()))
        .collect();

    let mut push = |value: String| {
        let value = value.trim().to_string();
        if value.chars().count() >= 3 && seen.insert(value.to_lowercase()) {
            variants.push(value);
        }
    };

    push(query.to_string());
    if !cleaned.is_empty() {
        push(cleaned.join(" "));
        for pair in cleaned.windows(2).take(3) {
            push(pair.join(" "));
        }
        if cleaned.len() > 2 {
            push(cleaned[0].clone());
        }
    }
    drop(push);
    variants.truncate(max_variants);
    variants
}

fn passage_key(hit: &crate::domain::document::SearchHit) -> String {
    format!("{}#{}", hit.path, hit.page.unwrap_or(0))
}

pub fn run(args: AnswerArgs, cfg: &AppConfig) -> Result<()> {
    let query = args.query.trim();
    if query.chars().count() < cfg.settings.answer.min_query_chars || !cfg.db_path.exists() {
        println!("{}", json!({"status":"no_context","answer":"","sources":[]}));
        return Ok(());
    }

    let model = args.model.as_deref().unwrap_or(&cfg.settings.answer.model);
    let limit = args.limit.unwrap_or(cfg.settings.answer.retrieval_limit).clamp(1, 16);
    let answer_cfg = &cfg.settings.answer;
    if !answer_cfg.provider.eq_ignore_ascii_case("ollama") {
        return Err(crate::error::HydrogenError::Config(format!("unsupported answer provider: {}", answer_cfg.provider)));
    }
    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let variants = query_variants(query, cfg.settings.answer.max_query_variants);
    let mut candidates: HashMap<String, (crate::domain::document::SearchHit, usize, usize)> =
        HashMap::new();

    // Search individual PDF pages, not just one best page per PDF. This is
    // essential when a definition appears in lecture notes far from the title.
    for variant in &variants {
        let page_hits = repo.search_pdf_passages(variant, answer_cfg.candidate_limit)?;
        for (rank, hit) in page_hits.into_iter().enumerate() {
            let key = passage_key(&hit);
            let entry = candidates.entry(key).or_insert_with(|| (hit.clone(), 0, rank));
            entry.1 += 1;
            entry.2 = entry.2.min(rank);
            if hit.snippet.len() > entry.0.snippet.len() {
                entry.0.snippet = hit.snippet;
            }
        }

        // Also retrieve ordinary indexed text documents (Markdown, TXT, DOCX
        // extracts, etc.). PDFs are handled by the page-level index above.
        let text_hits = repo.search(variant, answer_cfg.candidate_limit, "all")?;
        for (rank, hit) in text_hits.into_iter().enumerate() {
            if hit.is_dir
                || hit.source == "image"
                || Path::new(&hit.path)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "pdf" | "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"))
                    .unwrap_or(false)
                || hit.snippet.trim().is_empty()
                || (hit.coverage < 0.5 && !hit.phrase)
            {
                continue;
            }
            let key = passage_key(&hit);
            let entry = candidates.entry(key).or_insert_with(|| (hit.clone(), 0, rank));
            entry.1 += 1;
            entry.2 = entry.2.min(rank);
            if hit.snippet.len() > entry.0.snippet.len() {
                entry.0.snippet = hit.snippet;
            }
        }
    }

    let mut ranked: Vec<_> = candidates.into_values().collect();
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| b.0.coverage.total_cmp(&a.0.coverage))
            .then_with(|| b.0.score.total_cmp(&a.0.score))
    });
    ranked.truncate(limit);

    let mut sources = Vec::new();
    let mut context = String::new();
    for (index, (hit, _, _)) in ranked.into_iter().enumerate() {
        let id = index + 1;
        let location = hit
            .page
            .map(|page| format!("page {page}"))
            .or_else(|| hit.line.map(|line| format!("line {line}")))
            .unwrap_or_else(|| "indexed text".to_string());
        let snippet: String = hit.snippet.chars().take(answer_cfg.context_chars_per_passage).collect();
        if snippet.trim().is_empty() {
            continue;
        }
        context.push_str(&format!(
            "[{id}] {} ({location})\n{}\n\n",
            hit.path, snippet
        ));
        sources.push(json!({
            "id": id,
            "path": hit.path,
            "name": hit.name,
            "page": hit.page,
            "line": hit.line,
            "snippet": snippet
        }));
    }

    if sources.is_empty() {
        println!("{}", json!({"status":"no_context","answer":"","sources":[]}));
        return Ok(());
    }

    let prompt = format!(
        "You are a source-grounded study assistant. The excerpts below are the ONLY permitted evidence.\n\
         Answer the exact question, using the terminology and meaning in the supplied course notes.\n\
         Do not answer from general knowledge or memory. Do not infer a definition from a question that merely mentions the term.\n\
         If no excerpt actually explains or defines the requested concept, respond exactly: 'I cannot find an explanation in the indexed sources.'\n\
         Every factual sentence must end with a citation to the excerpt that directly supports it, such as [1]. Never invent citations.\n\
         If sources disagree or are incomplete, explicitly say so. Keep the answer concise.\n\n\
         QUESTION:\n{query}\n\nRETRIEVED COURSE EXCERPTS:\n{context}\nANSWER:"
    );
    let payload = json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "options": {
            "num_ctx": answer_cfg.context_window,
            "num_predict": answer_cfg.max_output_tokens,
            "temperature": answer_cfg.temperature,
            "top_p": answer_cfg.top_p,
            "repeat_penalty": answer_cfg.repeat_penalty
        }
    })
    .to_string();

    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail-with-body",
            "--connect-timeout",
            "2",
            "--max-time",
            &answer_cfg.timeout_seconds.to_string(),
            &format!("{}/api/generate", answer_cfg.endpoint.trim_end_matches('/')),
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            &payload,
        ])
        .output();

    let response = match output {
        Ok(output) if output.status.success() => serde_json::from_slice::<Value>(&output.stdout)
            .ok()
            .and_then(|value| {
                value
                    .get("response")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
        _ => None,
    };

    match response {
        Some(answer) if !answer.trim().is_empty() => {
            println!(
                "{}",
                json!({
                    "status": "ready",
                    "model": model,
                    "answer": answer.trim(),
                    "sources": sources
                })
            );
        }
        _ => {
            println!(
                "{}",
                json!({
                    "status": "unavailable",
                    "model": model,
                    "answer": "",
                    "sources": sources,
                    "error": format!("Could not reach Ollama at {} or the requested model is unavailable.", answer_cfg.endpoint)
                })
            );
        }
    }
    Ok(())
}
