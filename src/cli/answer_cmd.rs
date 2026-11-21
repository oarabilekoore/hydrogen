use crate::adapters::ml::embeddings::FastEmbedProvider;
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::application::query_service::QueryService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::Args;
use serde_json::{json, Value};
use std::process::Command;

#[derive(Args, Debug)]
pub struct AnswerArgs {
    /// Question to answer from indexed local documents
    pub query: String,
    /// Maximum number of retrieved passages used as context
    #[arg(long, default_value_t = 6)]
    pub limit: usize,
    /// Ollama model tag (defaults to the installed lightweight Gemma model)
    #[arg(long, default_value = "gemma3:1b")]
    pub model: String,
    /// Use enabled local MiniLM/CLIP vectors as additional retrieval channels.
    #[arg(long)]
    pub semantic: bool,
}

pub fn run(args: AnswerArgs, cfg: &AppConfig) -> Result<()> {
    let query = args.query.trim();
    if query.chars().count() < 3 {
        println!(
            "{}",
            json!({"status":"no_context","answer":"","sources":[]})
        );
        return Ok(());
    }
    if !cfg.db_path.exists() {
        println!(
            "{}",
            json!({"status":"no_context","answer":"","sources":[]})
        );
        return Ok(());
    }

    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let embeddings = FastEmbedProvider::new(cfg.models_dir.clone());
    let service = QueryService::new(&repo, &embeddings);
    // Keep RAG retrieval deterministic and cheap. It uses the same indexed
    // lexical/FTS5 passages as launcher search and does not download encoders
    // unless the existing semantic index has explicitly been enabled.
    let hits = service.search(query, args.limit.clamp(1, 8), "all", args.semantic)?;

    let mut sources = Vec::new();
    let mut context = String::new();
    for (index, hit) in hits
        .iter()
        .filter(|hit| !hit.snippet.trim().is_empty())
        .enumerate()
    {
        let id = index + 1;
        let location = hit
            .page
            .map(|page| format!("page {page}"))
            .or_else(|| hit.line.map(|line| format!("line {line}")))
            .unwrap_or_else(|| "location unavailable".to_string());
        let snippet: String = hit.snippet.chars().take(1400).collect();
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
        println!(
            "{}",
            json!({"status":"no_context","answer":"","sources":[]})
        );
        return Ok(());
    }

    let prompt = format!(
        "You answer questions using ONLY the supplied source excerpts.\nIf the excerpts do not contain enough information, say that the indexed documents do not provide enough information.\nDo not use outside knowledge, do not guess, and do not invent facts.\nWrite a concise, direct answer. Add inline source citations such as [1] after each supported claim.\n\nQUESTION:\n{query}\n\nRETRIEVED DOCUMENT EXCERPTS:\n{context}\nANSWER:"
    );
    let payload = json!({
        "model": &args.model,
        "prompt": prompt,
        "stream": false,
        "options": {
            "temperature": 0.1,
            "num_ctx": 3072,
            "num_predict": 220
        }
    });

    let payload = payload.to_string();
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail-with-body",
            "--connect-timeout",
            "2",
            "--max-time",
            "50",
            "http://127.0.0.1:11434/api/generate",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            &payload,
        ])
        .output();

    let response = match output {
        Ok(output) if output.status.success() => {
            serde_json::from_slice::<Value>(&output.stdout)
                .ok()
                .and_then(|value| {
                    value
                        .get("response")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
        }
        _ => None,
    };

    match response {
        Some(answer) if !answer.trim().is_empty() => {
            println!(
                "{}",
                json!({
                    "status": "ready",
                    "model": args.model,
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
                    "model": args.model,
                    "answer": "",
                    "sources": sources,
                    "error": "Could not reach Ollama at http://127.0.0.1:11434 or the requested model is unavailable."
                })
            );
        }
    }
    Ok(())
}
