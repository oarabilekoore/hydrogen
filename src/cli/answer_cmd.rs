use crate::adapters::sqlite::knowledge::KnowledgeStore;
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
    /// Write a formatted local HTML answer and include its path in the JSON response
    #[arg(long)]
    pub html: bool,
    /// Generate the HTML answer and open it in the default browser
    #[arg(long)]
    pub browser: bool,
}

fn query_variants(query: &str, max_variants: usize) -> Vec<String> {
    const STOP: &[&str] = &[
        "what", "which", "where", "when", "who", "why", "how", "is", "are", "was", "were", "the",
        "a", "an", "in", "on", "of", "for", "to", "and", "or", "with", "from", "explain",
        "describe", "define", "tell", "me", "about",
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

/// Extract the user-facing answer from both Ollama chat and legacy generate
/// response shapes. Deliberately never substitute the model's thinking field.
fn extract_ollama_answer(value: &Value) -> Option<String> {
    value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .or_else(|| value.get("response").and_then(Value::as_str))
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
        .map(str::to_owned)
}

fn ollama_empty_diagnostic(value: &Value) -> String {
    let done_reason = value
        .get("done_reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let thinking_chars = value
        .pointer("/message/thinking")
        .and_then(Value::as_str)
        .map(str::len)
        .unwrap_or(0);
    let eval_count = value.get("eval_count").and_then(Value::as_u64).unwrap_or(0);
    format!(
        "Ollama completed without answer text (done_reason={done_reason}, thinking_chars={thinking_chars}, eval_count={eval_count}). The thinking channel is not treated as an answer. Try a non-thinking model or increase answer.max_output_tokens."
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn source_file_url(path: &str, page: Option<u64>) -> String {
    let mut url = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    if let Some(page) = page {
        url.push_str(&format!("#page={page}"));
    }
    url
}

fn render_inline_markdown(value: &str) -> String {
    let parts: Vec<&str> = value.split("**").collect();
    let mut rendered = String::new();
    let mut bold = false;
    for (index, part) in parts.iter().enumerate() {
        rendered.push_str(&escape_html(part));
        if index + 1 < parts.len() {
            rendered.push_str(if bold { "</strong>" } else { "<strong>" });
            bold = !bold;
        }
    }
    rendered
}

fn render_markdown(value: &str) -> String {
    let mut html = String::new();
    let mut paragraph = Vec::new();
    let mut in_list = false;
    let mut in_code = false;
    let close_paragraph = |html: &mut String, paragraph: &mut Vec<String>| {
        if !paragraph.is_empty() {
            html.push_str("<p>");
            html.push_str(&paragraph.join(" "));
            html.push_str("</p>");
            paragraph.clear();
        }
    };
    for line in value.lines() {
        let trimmed = line.trim();
        if line.as_bytes().starts_with(&[96, 96, 96]) {
            close_paragraph(&mut html, &mut paragraph);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            if in_code {
                html.push_str("</code></pre>");
            } else {
                html.push_str("<pre><code>");
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            html.push_str(&escape_html(line));
            html.push('\n');
            continue;
        }
        if trimmed.is_empty() {
            close_paragraph(&mut html, &mut paragraph);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
        } else if let Some(heading) = trimmed.strip_prefix("### ") {
            close_paragraph(&mut html, &mut paragraph);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str(&format!("<h3>{}</h3>", render_inline_markdown(heading)));
        } else if let Some(heading) = trimmed.strip_prefix("## ") {
            close_paragraph(&mut html, &mut paragraph);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str(&format!("<h2>{}</h2>", render_inline_markdown(heading)));
        } else if let Some(heading) = trimmed.strip_prefix("# ") {
            close_paragraph(&mut html, &mut paragraph);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str(&format!("<h2>{}</h2>", render_inline_markdown(heading)));
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            close_paragraph(&mut html, &mut paragraph);
            if !in_list {
                html.push_str("<ul>");
                in_list = true;
            }
            html.push_str(&format!("<li>{}</li>", render_inline_markdown(item)));
        } else {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            paragraph.push(render_inline_markdown(trimmed));
        }
    }
    close_paragraph(&mut html, &mut paragraph);
    if in_list {
        html.push_str("</ul>");
    }
    if in_code {
        html.push_str("</code></pre>");
    }
    html
}

fn write_answer_html(
    cfg: &AppConfig,
    query: &str,
    answer: &str,
    model: &str,
    sources: &[Value],
) -> std::io::Result<String> {
    let mut source_html = String::new();
    for source in sources {
        let id = source.get("id").and_then(Value::as_u64).unwrap_or(0);
        let path = source.get("path").and_then(Value::as_str).unwrap_or("");
        let name = source.get("name").and_then(Value::as_str).unwrap_or(path);
        let page = source.get("page").and_then(Value::as_u64);
        let line = source.get("line").and_then(Value::as_u64);
        let source_url = source
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| source_file_url(path, page));
        let snippet = source.get("snippet").and_then(Value::as_str).unwrap_or("");
        let location = page
            .map(|n| format!("Page {n}"))
            .or_else(|| line.map(|n| format!("Line {n}")))
            .unwrap_or_else(|| "Indexed passage".to_string());
        source_html.push_str(&format!(
            "<article class=\"source\" id=\"source-{id}\"><div class=\"source-head\"><a href=\"{}\">{}</a><span>{}</span></div><p>{}</p><pre>{}</pre></article>",
            escape_html(&source_url),
            escape_html(name),
            escape_html(&location),
            escape_html(path),
            escape_html(snippet)
        ));
    }
    let html = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{} · Hydrogen Answer</title>
<style>
:root {{ color-scheme:dark; --bg:#101113; --panel:#191b1f; --text:#e8e8e8; --muted:#a0a5ad; --accent:#a8c7fa; --line:#30343b; }}
* {{ box-sizing:border-box; }} body {{ margin:0; background:var(--bg); color:var(--text); font:16px/1.72 Inter,system-ui,sans-serif; }}
main {{ width:min(900px,calc(100% - 36px)); margin:48px auto 80px; }} header {{ border-bottom:1px solid var(--line); padding-bottom:22px; margin-bottom:28px; }}
.eyebrow {{ color:var(--accent); text-transform:uppercase; letter-spacing:.12em; font-size:12px; font-weight:700; }}
h1 {{ font-size:clamp(25px,4vw,38px); line-height:1.2; margin:10px 0; }} .meta {{ color:var(--muted); font-size:13px; }}
.answer {{ background:var(--panel); border:1px solid var(--line); border-radius:14px; padding:clamp(20px,4vw,36px); overflow-wrap:anywhere; }}
.answer p {{ margin:0 0 16px; }} .answer p:last-child {{ margin-bottom:0; }} .answer h2,.answer h3 {{ line-height:1.3; margin:26px 0 12px; }}
.answer ul {{ padding-left:24px; }} .answer li {{ margin:6px 0; }} .answer pre {{ overflow:auto; padding:16px; background:#0b0c0e; border-radius:8px; white-space:pre-wrap; }}
.sources {{ margin-top:38px; }} .source {{ background:var(--panel); border:1px solid var(--line); border-radius:10px; padding:16px 18px; margin:12px 0; }}
.source-head {{ display:flex; flex-wrap:wrap; gap:12px; justify-content:space-between; }} a {{ color:var(--accent); text-decoration:none; }} a:hover {{ text-decoration:underline; }}
.source-head span {{ color:var(--muted); font-size:13px; }} .source p {{ color:var(--muted); font-size:12px; overflow-wrap:anywhere; }}
.source pre {{ white-space:pre-wrap; overflow-wrap:anywhere; font:13px/1.6 ui-monospace,monospace; }}
footer {{ color:var(--muted); font-size:12px; margin-top:32px; }}
</style></head><body><main>
<header><div class="eyebrow">Hydrogen · Local RAG</div><h1>{}</h1><div class="meta">Generated locally with {} · Based on {} retrieved source passages</div></header>
<section class="answer">{}</section><section class="sources"><h2>Sources</h2>{}</section>
<footer>This answer was generated from the indexed local passages listed above. Verify important details against the original documents.</footer>
</main></body></html>"#,
        escape_html(query),
        escape_html(query),
        escape_html(model),
        sources.len(),
        render_markdown(answer),
        source_html
    );
    let directory = cfg.state_dir.join("answers");
    std::fs::create_dir_all(&directory)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let digest = blake3::hash(query.as_bytes()).to_hex();
    let path = directory.join(format!("answer-{stamp}-{}.html", &digest[..10]));
    std::fs::write(&path, html)?;
    Ok(path.to_string_lossy().into_owned())
}

pub fn run(args: AnswerArgs, cfg: &AppConfig) -> Result<()> {
    let query = args.query.trim();
    if query.chars().count() < cfg.settings.answer.min_query_chars || !cfg.db_path.exists() {
        println!(
            "{}",
            json!({"status":"no_context","answer":"","sources":[]})
        );
        return Ok(());
    }

    let model = args.model.as_deref().unwrap_or(&cfg.settings.answer.model);
    let limit = args
        .limit
        .unwrap_or(cfg.settings.answer.retrieval_limit)
        .clamp(1, 16);
    let answer_cfg = &cfg.settings.answer;
    if !answer_cfg.provider.eq_ignore_ascii_case("ollama") {
        return Err(crate::error::HydrogenError::Config(format!(
            "unsupported answer provider: {}",
            answer_cfg.provider
        )));
    }
    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let variants = query_variants(query, cfg.settings.answer.max_query_variants);
    let mut candidates: HashMap<String, (crate::domain::document::SearchHit, usize, usize)> =
        HashMap::new();

    // Search individual PDF pages, not just one best page per PDF. This is
    // essential when a definition appears in lecture notes far from the title.
    for variant in &variants {
        let page_hits = if cfg.settings.indexing.pdf.enabled {
            repo.search_pdf_passages(variant, answer_cfg.candidate_limit)?
        } else {
            Vec::new()
        };
        for (rank, hit) in page_hits.into_iter().enumerate() {
            let key = passage_key(&hit);
            let entry = candidates
                .entry(key)
                .or_insert_with(|| (hit.clone(), 0, rank));
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
                    .map(|ext| {
                        matches!(
                            ext.to_ascii_lowercase().as_str(),
                            "pdf" | "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
                        )
                    })
                    .unwrap_or(false)
                || hit.snippet.trim().is_empty()
                || (hit.coverage < 0.5 && !hit.phrase)
            {
                continue;
            }
            let key = passage_key(&hit);
            let entry = candidates
                .entry(key)
                .or_insert_with(|| (hit.clone(), 0, rank));
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
        let snippet: String = hit
            .snippet
            .chars()
            .take(answer_cfg.context_chars_per_passage)
            .collect();
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

    // Include saved web articles when the local file index did not fill the
    // configured retrieval budget. Captured pages remain offline and searchable.
    if sources.len() < limit {
        let knowledge = KnowledgeStore::open(&cfg.db_path).map_err(|error| {
            crate::error::HydrogenError::Config(format!("could not open saved knowledge: {error}"))
        })?;
        let remaining = limit.saturating_sub(sources.len());
        for page in knowledge.search_web_pages(query, remaining)? {
            let id = sources.len() + 1;
            let snippet: String = page
                .content
                .chars()
                .take(answer_cfg.context_chars_per_passage)
                .collect();
            if snippet.trim().is_empty() {
                continue;
            }
            context.push_str(&format!(
                "[{id}] {} ({})\\n{}\\n\\n",
                page.title, page.url, snippet
            ));
            sources.push(json!({
                "id": id,
                "kind": "web_page",
                "path": "",
                "name": page.title,
                "url": page.url,
                "page": null,
                "line": null,
                "snippet": snippet
            }));
        }
    }

    if sources.is_empty() {
        println!(
            "{}",
            json!({"status":"no_context","answer":"","sources":[]})
        );
        return Ok(());
    }

    let prompt = format!(
        "You are a source-grounded study assistant. The excerpts below are the ONLY permitted evidence.\n\
         Answer the exact question, using the terminology and meaning in the supplied course notes.\n\
         Do not answer from general knowledge or memory. Do not infer a definition from a question that merely mentions the term.\n\
         If no excerpt actually explains or defines the requested concept, respond exactly: 'I cannot find an explanation in the indexed sources.'\n\
         Every factual sentence must end with a citation to the excerpt that directly supports it, such as [1]. Never invent citations.\n\
         If sources disagree or are incomplete, explicitly say so. Keep the answer concise.\n\n\
         QUESTION:\n{query}\n\nRETRIEVED INDEXED EXCERPTS:\n{context}\nANSWER:"
    );
    // Use /api/chat and explicitly disable thinking for this source-grounded
    // answer path. Small local models can otherwise spend the entire output
    // budget in a private reasoning channel and return an empty final answer.
    let payload = json!({
        "model": model,
        "messages": [
            {
                "role": "user",
                "content": prompt
            }
        ],
        "stream": false,
        "think": false,
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
            &format!("{}/api/chat", answer_cfg.endpoint.trim_end_matches('/')),
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            &payload,
        ])
        .output();

    let (response, generation_error) =
        match output {
            Ok(output) => {
                let body = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let parsed = serde_json::from_str::<Value>(&body).ok();
                if output.status.success() {
                    let answer = parsed.as_ref().and_then(extract_ollama_answer);
                    let error =
                        if answer.as_deref().unwrap_or("").trim().is_empty() {
                            Some(parsed.as_ref().map(ollama_empty_diagnostic).unwrap_or_else(
                                || {
                                    format!(
                                        "Ollama returned a non-JSON response: {}",
                                        body.chars().take(240).collect::<String>()
                                    )
                                },
                            ))
                        } else {
                            None
                        };
                    (answer, error)
                } else {
                    let api_error = parsed
                        .as_ref()
                        .and_then(|value| value.get("error"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let detail = api_error
                        .or_else(|| {
                            if !stderr.is_empty() {
                                Some(stderr)
                            } else if !body.trim().is_empty() {
                                Some(body.trim().to_string())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| format!("curl exited with {}", output.status));
                    (
                        None,
                        Some(format!(
                            "Ollama request to {} failed: {}",
                            answer_cfg.endpoint, detail
                        )),
                    )
                }
            }
            Err(error) => (
                None,
                Some(format!("Could not start curl for Ollama request: {error}")),
            ),
        };

    match response {
        Some(answer) if !answer.trim().is_empty() => {
            let answer = answer.trim();
            let html_path = if args.html || args.browser {
                Some(
                    write_answer_html(cfg, query, answer, model, &sources).map_err(|error| {
                        crate::error::HydrogenError::Config(format!(
                            "could not write local answer page: {error}"
                        ))
                    })?,
                )
            } else {
                None
            };
            let browser_opened = if args.browser {
                if let Some(path) = html_path.as_deref() {
                    match Command::new("xdg-open").arg(path).spawn() {
                        Ok(_) => true,
                        Err(error) => {
                            eprintln!("Hydrogen could not open the answer in a browser: {error}");
                            false
                        }
                    }
                } else {
                    false
                }
            } else {
                false
            };
            println!(
                "{}",
                json!({
                    "status": "ready", "model": model, "answer": answer,
                    "sources": sources, "html_path": html_path,
                    "browser_opened": browser_opened
                })
            );
        }
        _ => {
            println!(
                "{}",
                json!({
                    "status": "unavailable", "model": model, "answer": "",
                    "sources": sources,
                    "error": generation_error.unwrap_or_else(|| "Ollama returned no answer.".to_string())
                })
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod html_tests {
    use super::*;

    #[test]
    fn extracts_ollama_chat_content() {
        let value = json!({"message":{"role":"assistant","content":"  grounded answer  "}});
        assert_eq!(
            extract_ollama_answer(&value).as_deref(),
            Some("grounded answer")
        );
    }

    #[test]
    fn extracts_legacy_generate_content() {
        let value = json!({"response":"legacy answer"});
        assert_eq!(
            extract_ollama_answer(&value).as_deref(),
            Some("legacy answer")
        );
    }

    #[test]
    fn does_not_mistake_thinking_for_an_answer() {
        let value = json!({
            "message":{"content":"","thinking":"private reasoning"},
            "done_reason":"length",
            "eval_count":180
        });
        assert_eq!(extract_ollama_answer(&value), None);
        let diagnostic = ollama_empty_diagnostic(&value);
        assert!(diagnostic.contains("thinking_chars=16"));
        assert!(diagnostic.contains("done_reason=length"));
    }

    #[test]
    fn html_escaping_prevents_markup_injection() {
        assert_eq!(
            escape_html("<script a=\"x\">"),
            "&lt;script a=&quot;x&quot;&gt;"
        );
    }

    #[test]
    fn source_links_preserve_pdf_page_and_escape_spaces() {
        assert_eq!(
            source_file_url("/home/test/My Notes/lecture.pdf", Some(14)),
            "file:///home/test/My%20Notes/lecture.pdf#page=14"
        );
    }
}
