use crate::adapters::sqlite::knowledge::KnowledgeStore;
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::config::AppConfig;
use crate::error::Result;
use crate::ports::repository::IndexRepository;
use clap::Args;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

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
    /// Reuse an existing conversation as multi-turn model context
    #[arg(long)]
    pub conversation: Option<String>,
    /// Emit incremental Ollama token events as newline-delimited JSON
    #[arg(long, hide = true)]
    pub stream: bool,
    /// Do not create a saved conversation (used by the frontend chat worker)
    #[arg(long, hide = true)]
    pub no_save: bool,
    /// Allow general model knowledge instead of restricting answers to retrieved excerpts
    #[arg(long)]
    pub no_grounding: bool,
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

fn recent_history_messages(mut history: Vec<Value>, query: &str) -> Vec<Value> {
    history.retain(|message| matches!(message.get("role").and_then(Value::as_str), Some("user") | Some("assistant")));
    if history.last().and_then(|message| message.get("role")).and_then(Value::as_str) == Some("user")
        && history.last().and_then(|message| message.get("content")).and_then(Value::as_str) == Some(query) {
        history.pop();
    }
    if history.len() > 6 { history.drain(..history.len() - 6); }
    for message in &mut history {
        if let Some(content) = message.get("content").and_then(Value::as_str).map(|value| value.chars().take(1500).collect::<String>()) {
            message["content"] = Value::String(content);
        }
    }
    history
}

fn visible_stream_text(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let mut output = String::new();
    let mut cursor = 0;
    let mut in_think = false;
    while cursor < raw.len() {
        if in_think {
            if let Some(relative) = lower[cursor..].find("</think>") {
                cursor += relative + "</think>".len();
                in_think = false;
            } else { break; }
        } else if let Some(relative) = lower[cursor..].find("<think>") {
            let start = cursor + relative;
            output.push_str(&raw[cursor..start]);
            cursor = start + "<think>".len();
            in_think = true;
        } else {
            let held = (1..=6).rev().find(|length| lower.as_bytes().ends_with(&b"<think>"[..*length]));
            let end = held.map(|length| raw.len() - length).unwrap_or(raw.len());
            output.push_str(&raw[cursor..end]);
            break;
        }
    }
    output
}

fn stream_ollama_chat(endpoint: &str, payload: &str, timeout_seconds: u64, mut on_delta: impl FnMut(&str), mut on_thinking: impl FnMut(usize)) -> std::result::Result<String, String> {
    let mut child = Command::new("curl")
        .args(["--silent", "--show-error", "--fail-with-body", "--no-buffer", "--connect-timeout", "2", "--max-time", &timeout_seconds.to_string(), &format!("{}/api/chat", endpoint.trim_end_matches('/')), "-H", "Content-Type: application/json", "--data-binary", payload])
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|error| format!("Could not start curl for Ollama request: {error}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "could not read Ollama stream".to_string())?;
    let reader = BufReader::new(stdout);
    let mut raw = String::new();
    let mut emitted = 0usize;
    let mut thinking_chars = 0usize;
    let mut stream_error = None;
    for line in reader.lines() {
        let line = line.map_err(|error| format!("could not read Ollama stream: {error}"))?;
        if line.trim().is_empty() { continue; }
        let value: Value = serde_json::from_str(&line).map_err(|error| format!("Ollama returned invalid streaming JSON: {error}"))?;
        if let Some(error) = value.get("error").and_then(Value::as_str) { stream_error = Some(error.to_string()); break; }
        if let Some(thinking) = value.pointer("/message/thinking").and_then(Value::as_str) {
            thinking_chars = thinking_chars.saturating_add(thinking.chars().count());
            on_thinking(thinking_chars);
        }
        if let Some(token) = value.pointer("/message/content").and_then(Value::as_str) {
            raw.push_str(token);
            let visible = visible_stream_text(&raw);
            if visible.len() > emitted { on_delta(&visible[emitted..]); emitted = visible.len(); }
        }
        if value.get("done").and_then(Value::as_bool) == Some(true) { break; }
    }
    let output = child.wait_with_output().map_err(|error| format!("could not finish Ollama stream: {error}"))?;
    if let Some(error) = stream_error { return Err(format!("Ollama request failed: {error}")); }
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("Ollama streaming request failed ({}): {}", output.status, if detail.is_empty() { "no diagnostic returned" } else { &detail }));
    }
    let answer = strip_think_blocks(&raw);
    if answer.is_empty() {
        let observed_thinking = if raw.is_empty() { thinking_chars } else { raw.chars().count() };
        return Err(format!("Ollama completed without answer text (thinking_chars={observed_thinking}). The thinking channel is not treated as an answer."));
    }
    Ok(answer)
}

fn ollama_show(endpoint: &str, model: &str, timeout_seconds: u64) -> Option<Value> {
    let payload = json!({"model":model}).to_string();
    let output = Command::new("curl")
        .args(["--silent", "--show-error", "--fail", "--connect-timeout", "2", "--max-time", &timeout_seconds.to_string(), &format!("{}/api/show", endpoint.trim_end_matches('/')), "-H", "Content-Type: application/json", "--data-binary", &payload])
        .output().ok()?;
    if !output.status.success() { return None; }
    serde_json::from_slice(&output.stdout).ok()
}

fn model_thinking_mode(show: Option<&Value>) -> Option<Value> {
    let metadata = show?;
    if let Some(values) = metadata.pointer("/thinking/values").and_then(Value::as_array) {
        if values.iter().any(|value| value.as_str() == Some("low")) { return Some(json!("low")); }
        if values.iter().any(|value| value.as_bool() == Some(true)) { return Some(json!(true)); }
    }
    let supported = metadata.get("capabilities").and_then(Value::as_array)
        .map(|values| values.iter().any(|value| value.as_str() == Some("thinking"))).unwrap_or(false);
    supported.then_some(json!(true))
}

fn model_supports_thinking(show: Option<&Value>) -> bool { model_thinking_mode(show).is_some() }

fn model_supports_no_thinking(show: Option<&Value>) -> bool {
    show.and_then(|value| value.pointer("/thinking/values"))
        .and_then(Value::as_array)
        .map(|values| values.iter().any(|value| value.as_bool() == Some(false)))
        .unwrap_or(false)
}

fn think_option_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    (lower.contains("think") || lower.contains("thinking"))
        && (lower.contains("unsupported") || lower.contains("not support") || lower.contains("unknown field")
            || lower.contains("invalid") || lower.contains("400") || lower.contains("bad request"))
}

fn increase_output_tokens(payload: &mut Value, multiplier: u64) {
    let current = payload.pointer("/options/num_predict").and_then(Value::as_u64).unwrap_or(180);
    let increased = current.saturating_mul(multiplier).max(512).min(2048.max(current));
    payload["options"]["num_predict"] = json!(increased);
}

fn request_ollama_chat(endpoint: &str, payload: &str, timeout_seconds: u64) -> std::result::Result<Value, String> {
    let output = Command::new("curl")
        .args(["--silent", "--show-error", "--fail-with-body", "--connect-timeout", "2", "--max-time", &timeout_seconds.to_string(), &format!("{}/api/chat", endpoint.trim_end_matches('/')), "-H", "Content-Type: application/json", "--data-binary", payload])
        .output()
        .map_err(|error| format!("Could not start curl for Ollama request: {error}"))?;
    let body = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let parsed = serde_json::from_str::<Value>(&body).ok();
    if output.status.success() {
        return parsed.ok_or_else(|| format!("Ollama returned a non-JSON response: {}", body.chars().take(240).collect::<String>()));
    }
    let api_error = parsed.as_ref().and_then(|value| value.get("error")).and_then(Value::as_str);
    let detail = api_error.map(str::to_owned)
        .or_else(|| (!stderr.is_empty()).then_some(stderr))
        .or_else(|| (!body.trim().is_empty()).then(|| body.trim().to_string()))
        .unwrap_or_else(|| format!("curl exited with {}", output.status));
    Err(format!("Ollama request failed ({}): {}", output.status, detail))
}

fn passage_key(hit: &crate::domain::document::SearchHit) -> String {
    format!("{}#{}", hit.path, hit.page.unwrap_or(0))
}

/// Extract the user-facing answer from both Ollama chat and legacy generate
/// response shapes. Deliberately never substitute the model's thinking field.
fn strip_think_blocks(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    let mut in_think = false;
    loop {
        let lower = rest.to_ascii_lowercase();
        if in_think {
            if let Some(end) = lower.find("</think>") {
                rest = &rest[end + "</think>".len()..];
                in_think = false;
            } else {
                break;
            }
        } else if let Some(start) = lower.find("<think>") {
            output.push_str(&rest[..start]);
            rest = &rest[start + "<think>".len()..];
            in_think = true;
        } else {
            output.push_str(rest);
            break;
        }
    }
    output.trim().to_string()
}

/// Extract user-facing answer text from Ollama chat and legacy generate responses.
/// Thinking blocks are never exposed as answer text.
fn extract_ollama_answer(value: &Value) -> Option<String> {
    value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .or_else(|| value.get("response").and_then(Value::as_str))
        .map(strip_think_blocks)
        .filter(|answer| !answer.is_empty())
}

fn has_thinking_output(value: &Value) -> bool {
    let separate = value.pointer("/message/thinking")
        .and_then(Value::as_str)
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false);
    let content = value.pointer("/message/content").and_then(Value::as_str).unwrap_or("");
    separate || (!content.is_empty() && strip_think_blocks(content).is_empty()
        && content.to_ascii_lowercase().contains("<think>"))
}

fn ollama_empty_diagnostic(value: &Value) -> String {
    let done_reason = value
        .get("done_reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let separate_thinking_chars = value
        .pointer("/message/thinking")
        .and_then(Value::as_str)
        .map(str::len)
        .unwrap_or(0);
    let content = value.pointer("/message/content").and_then(Value::as_str).unwrap_or("");
    let thinking_chars = if separate_thinking_chars > 0 { separate_thinking_chars } else if has_thinking_output(value) { content.chars().count() } else { 0 };
    let eval_count = value.get("eval_count").and_then(Value::as_u64).unwrap_or(0);
    format!(
        "Ollama completed without answer text (done_reason={done_reason}, thinking_chars={thinking_chars}, eval_count={eval_count}). The thinking channel is not treated as an answer. Try a non-thinking model or increase answer.max_output_tokens."
    )
}

fn thinking_only_stream_error(error: &str) -> bool {
    error.split("thinking_chars=").nth(1)
        .and_then(|value| value.split(|ch: char| !ch.is_ascii_digit()).next())
        .and_then(|value| value.parse::<usize>().ok())
        .map(|count| count > 0)
        .unwrap_or(false)
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
<title>{} · Good Search Answer</title>
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
<header><div class="eyebrow">Good Search · Local RAG</div><h1>{}</h1><div class="meta">Generated locally with {} · Based on {} retrieved source passages</div></header>
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

fn build_answer_prompts(query: &str, context: &str, grounding: bool) -> (&'static str, String) {
    if grounding {
        (
            "You are a source-grounded study assistant. Conversation history helps resolve references, but only the current retrieved excerpts may support factual claims. Do not treat earlier assistant messages as evidence.",
            format!(
                "You are a source-grounded study assistant. The excerpts below are the ONLY permitted evidence.\n\
                 Answer the exact question using only the terminology and meaning supported by the supplied indexed sources.\n\
                 Do not answer from general knowledge or memory. Do not infer a definition from a question that merely mentions the term.\n\
                 If no excerpt actually explains or defines the requested concept, respond exactly: 'I cannot find an explanation in the indexed sources.'\n\
                 Every factual sentence must end with a citation to the excerpt that directly supports it, such as [1]. Never invent citations.\n\
                 If sources disagree or are incomplete, explicitly say so. Keep the answer concise.\n\n\
                 QUESTION:\n{query}\n\nRETRIEVED INDEXED EXCERPTS:\n{context}\nANSWER:"
            )
        )
    } else {
        (
            "You are a helpful assistant. Answer using your general knowledge and reasoning. Retrieved indexed excerpts, if present, are optional context and must not be treated as complete or authoritative. Cite an excerpt as [n] only when it directly supports a claim; never invent citations. Be clear when you are uncertain.",
            format!(
                "Answer the question directly. You may use general knowledge even when the local index has no matching material. If retrieved excerpts are relevant, use them as optional supporting context and cite them accurately.\n\nQUESTION:\n{query}\n\nOPTIONAL LOCAL EXCERPTS:\n{}\nANSWER:",
                if context.trim().is_empty() { "No matching indexed excerpts were found." } else { context }
            )
        )
    }
}

pub fn run(args: AnswerArgs, cfg: &AppConfig) -> Result<()> {
    let query = args.query.trim();
    if query.chars().count() < cfg.settings.answer.min_query_chars || (!cfg.db_path.exists() && !args.no_grounding) {
        let result = json!({"status":"no_context","answer":"","sources":[]});
        if args.stream { println!("{}", json!({"event":"result","result":result})); }
        else { println!("{}", result); }
        return Ok(());
    }

    let model = args.model.as_deref().unwrap_or(&cfg.settings.answer.model);
    let limit = args
        .limit
        .unwrap_or(cfg.settings.answer.retrieval_limit)
        .clamp(1, 16);
    let answer_cfg = &cfg.settings.answer;
    if !answer_cfg.provider.eq_ignore_ascii_case("ollama") {
        return Err(crate::error::GoodSearchError::Config(format!(
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
            crate::error::GoodSearchError::Config(format!("could not open saved knowledge: {error}"))
        })?;
        let remaining = limit.saturating_sub(sources.len());
        for page in knowledge.search_web_pages(query, remaining)? {
            let id = sources
                .iter()
                .filter_map(|source| source.get("id").and_then(Value::as_u64))
                .max()
                .unwrap_or(0)
                + 1;
            let snippet: String = page
                .content
                .chars()
                .take(answer_cfg.context_chars_per_passage)
                .collect();
            if snippet.trim().is_empty() {
                continue;
            }
            context.push_str(&format!(
                "[{id}] {} ({})\n{}\n\n",
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

    if sources.is_empty() && !args.no_grounding {
        let result = json!({"status":"no_context","answer":"","sources":[]});
        if args.stream { println!("{}", json!({"event":"result","result":result})); }
        else { println!("{}", result); }
        return Ok(());
    }

    // Recent turns help resolve references; grounding mode controls whether retrieved excerpts are required.
    let stored_history: Vec<Value> = if let Some(conversation_id) = args.conversation.as_deref() {
        KnowledgeStore::open(&cfg.db_path)
            .and_then(|knowledge| knowledge.get_messages(conversation_id))
            .unwrap_or_default()
            .into_iter()
            .map(|message| json!({"role":message.role,"content":message.content}))
            .collect()
    } else { Vec::new() };
    let history = recent_history_messages(stored_history, query);
    let (system_prompt, prompt) = build_answer_prompts(query, &context, !args.no_grounding);
    let mut messages = vec![json!({
        "role":"system",
        "content":system_prompt
    })];
    messages.extend(history);
    messages.push(json!({"role":"user","content":prompt}));

    // Ollama exposes per-model capabilities through /api/show. Only enable
    // thinking when the selected model explicitly advertises that capability.
    let show = ollama_show(&answer_cfg.endpoint, model, answer_cfg.timeout_seconds.min(5));
    let thinking_mode = model_thinking_mode(show.as_ref());
    let thinking_supported = thinking_mode.is_some();
    let mut payload_value = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "options": {
            "num_ctx": answer_cfg.context_window,
            "num_predict": if thinking_supported { answer_cfg.max_output_tokens.max(1024) } else { answer_cfg.max_output_tokens },
            "temperature": answer_cfg.temperature,
            "top_p": answer_cfg.top_p,
            "repeat_penalty": answer_cfg.repeat_penalty
        }
    });
    if let Some(mode) = thinking_mode { payload_value["think"] = mode; }

    let (response, generation_error) = if args.stream {
        payload_value["stream"] = json!(true);
        let mut emit = |text: &str| { println!("{}", json!({"event":"delta","text":text})); let _ = std::io::stdout().flush(); };
        let mut emit_thinking = |chars: usize| { println!("{}", json!({"event":"thinking","chars":chars})); let _ = std::io::stdout().flush(); };
        let payload = payload_value.to_string();
        match stream_ollama_chat(&answer_cfg.endpoint, &payload, answer_cfg.timeout_seconds, &mut emit, &mut emit_thinking) {
            Ok(answer) => (Some(answer), None),
            Err(error) if payload_value.get("think").is_some() && think_option_error(&error) => {
                payload_value.as_object_mut().map(|object| object.remove("think"));
                match stream_ollama_chat(&answer_cfg.endpoint, &payload_value.to_string(), answer_cfg.timeout_seconds, &mut emit, &mut emit_thinking) {
                    Ok(answer) => (Some(answer), None),
                    Err(retry_error) => (None, Some(format!("{error}; retry without thinking also failed: {retry_error}"))),
                }
            }
            Err(error) if thinking_supported && thinking_only_stream_error(&error) => {
                increase_output_tokens(&mut payload_value, 2);
                match stream_ollama_chat(&answer_cfg.endpoint, &payload_value.to_string(), answer_cfg.timeout_seconds, &mut emit, &mut emit_thinking) {
                    Ok(answer) => (Some(answer), None),
                    Err(retry_error) if thinking_only_stream_error(&retry_error) && model_supports_no_thinking(show.as_ref()) => {
                        // Preserve Qwen reasoning on the first attempt. If it consumes
                        // the entire output budget twice, make one answer-only fallback.
                        payload_value["think"] = json!(false);
                        increase_output_tokens(&mut payload_value, 2);
                        match stream_ollama_chat(&answer_cfg.endpoint, &payload_value.to_string(), answer_cfg.timeout_seconds, &mut emit, &mut emit_thinking) {
                            Ok(answer) => (Some(answer), None),
                            Err(final_error) => (None, Some(format!("{error}; larger reasoning budget failed: {retry_error}; answer-only fallback failed: {final_error}"))),
                        }
                    }
                    Err(retry_error) => (None, Some(format!("{error}; retry with a larger output budget failed: {retry_error}"))),
                }
            }
            Err(error) => (None, Some(error)),
        }
    } else {
        let first = request_ollama_chat(&answer_cfg.endpoint, &payload_value.to_string(), answer_cfg.timeout_seconds);
        let first = match first {
            Err(error) if payload_value.get("think").is_some() && think_option_error(&error) => {
                payload_value.as_object_mut().map(|object| object.remove("think"));
                request_ollama_chat(&answer_cfg.endpoint, &payload_value.to_string(), answer_cfg.timeout_seconds)
                    .map_err(|retry| format!("{error}; retry without thinking also failed: {retry}"))
            }
            other => other,
        };
        match first {
            Ok(value) => {
                let answer = extract_ollama_answer(&value);
                if answer.as_deref().unwrap_or("").trim().is_empty()
                    && thinking_supported
                    && has_thinking_output(&value) {
                    increase_output_tokens(&mut payload_value, 2);
                    match request_ollama_chat(&answer_cfg.endpoint, &payload_value.to_string(), answer_cfg.timeout_seconds) {
                        Ok(retry) => {
                            let retry_answer = extract_ollama_answer(&retry);
                            if retry_answer.as_deref().unwrap_or("").trim().is_empty()
                                && model_supports_no_thinking(show.as_ref()) {
                                // The reasoning attempt remains the default. Fall back
                                // to answer-only generation only after it exhausts its budget.
                                payload_value["think"] = json!(false);
                                increase_output_tokens(&mut payload_value, 2);
                                match request_ollama_chat(&answer_cfg.endpoint, &payload_value.to_string(), answer_cfg.timeout_seconds) {
                                    Ok(final_retry) => {
                                        let final_answer = extract_ollama_answer(&final_retry);
                                        if final_answer.as_deref().unwrap_or("").trim().is_empty() {
                                            (None, Some(ollama_empty_diagnostic(&final_retry)))
                                        } else { (final_answer, None) }
                                    }
                                    Err(error) => (None, Some(format!("Qwen reasoning exhausted its output budget; answer-only fallback failed: {error}"))),
                                }
                            } else if retry_answer.as_deref().unwrap_or("").trim().is_empty() {
                                (None, Some(ollama_empty_diagnostic(&retry)))
                            } else { (retry_answer, None) }
                        }
                        Err(error) => (None, Some(format!("Ollama used its thinking budget without producing answer text; retry failed: {error}"))),
                    }
                } else if answer.as_deref().unwrap_or("").trim().is_empty() {
                    (None, Some(ollama_empty_diagnostic(&value)))
                } else {
                    (answer, None)
                }
            }
            Err(error) => (None, Some(error)),
        }
    };

    match response {
        Some(answer) if !answer.trim().is_empty() => {
            let answer = answer.trim();
            let html_path = if args.html || args.browser {
                Some(
                    write_answer_html(cfg, query, answer, model, &sources).map_err(|error| {
                        crate::error::GoodSearchError::Config(format!(
                            "could not write local answer page: {error}"
                        ))
                    })?,
                )
            } else {
                None
            };
            let conversation_id = if args.no_save {
                None
            } else {
                let knowledge = KnowledgeStore::open(&cfg.db_path).map_err(|error| {
                    crate::error::GoodSearchError::Config(format!("could not save answer: {error}"))
                })?;
                let id = knowledge.create_conversation(query)?;
                knowledge.append_message(&id, "user", query, None)?;
                knowledge.append_message_with_sources(
                    &id,
                    "assistant",
                    answer,
                    Some(model),
                    Some(&json!(sources)),
                )?;
                Some(id)
            };
            let browser_opened = if args.browser {
                if let Some(path) = html_path.as_deref() {
                    match Command::new("xdg-open").arg(path).spawn() {
                        Ok(_) => true,
                        Err(error) => {
                            eprintln!("Good Search could not open the answer in a browser: {error}");
                            false
                        }
                    }
                } else {
                    false
                }
            } else {
                false
            };
            let result = json!({
                "status": "ready", "model": model, "answer": answer,
                "sources": sources, "html_path": html_path,
                "browser_opened": browser_opened,
                "conversation_id": conversation_id
            });
            if args.stream { println!("{}", json!({"event":"result","result":result})); }
            else { println!("{}", result); }
        }
        _ => {
            let result = json!({
                "status": "unavailable", "model": model, "answer": "",
                "sources": sources,
                "error": generation_error.unwrap_or_else(|| "Ollama returned no answer.".to_string())
            });
            if args.stream { println!("{}", json!({"event":"result","result":result})); }
            else { println!("{}", result); }
        }
    }
    Ok(())
}

#[cfg(test)]
mod html_tests {
    use super::*;

    #[test]
    fn grounding_mode_changes_the_answer_evidence_policy() {
        let (grounded_system, grounded_prompt) =
            build_answer_prompts("Define entropy", "[1] Lecture notes\\nEntropy is a state function.", true);
        assert!(grounded_system.contains("only the current retrieved excerpts"));
        assert!(grounded_prompt.contains("ONLY permitted evidence"));
        assert!(grounded_prompt.contains("[1] Lecture notes"));

        let (general_system, general_prompt) =
            build_answer_prompts("What is entropy?", "", false);
        assert!(general_system.contains("general knowledge and reasoning"));
        assert!(general_prompt.contains("You may use general knowledge"));
        assert!(general_prompt.contains("No matching indexed excerpts were found."));
        assert!(!general_prompt.contains("ONLY permitted evidence"));
    }

    #[test]
    fn conversation_history_excludes_the_current_user_turn_and_bounds_context() {
        let history = vec![
            json!({"role":"user","content":"first"}),
            json!({"role":"assistant","content":"reply"}),
            json!({"role":"user","content":"current question"}),
        ];
        let prepared = recent_history_messages(history, "current question");
        assert_eq!(prepared.len(), 2);
        assert_eq!(prepared[0]["content"], "first");
        let long = "x".repeat(2000);
        let prepared = recent_history_messages(vec![json!({"role":"assistant","content":long})], "next");
        assert_eq!(prepared[0]["content"].as_str().unwrap().chars().count(), 1500);
    }

    #[test]
    fn streaming_hides_partial_and_complete_thinking_blocks() {
        assert_eq!(visible_stream_text("Answer <thi"), "Answer ");
        assert_eq!(visible_stream_text("Answer <think>secret"), "Answer ");
        assert_eq!(visible_stream_text("Answer <think>secret</think> done"), "Answer  done");
        assert_eq!(visible_stream_text("Visible"), "Visible");
    }

    #[test]
    fn streaming_withholds_partial_think_tags_without_splitting_utf8() {
        assert_eq!(visible_stream_text("café <thi"), "café ");
        assert_eq!(visible_stream_text("Answer <think>hidden</think> after"), "Answer  after");
        assert_eq!(visible_stream_text("visible <th"), "visible ");
    }

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
    fn strips_think_blocks_without_removing_surrounding_answer() {
        let value = json!({"message":{"content":"Before. <think>private reasoning</think> After."}});
        assert_eq!(extract_ollama_answer(&value).as_deref(), Some("Before.  After."));
    }

    #[test]
    fn ignores_a_response_that_contains_only_a_think_block() {
        let value = json!({"response":"<think>private reasoning</think>"});
        assert_eq!(extract_ollama_answer(&value), None);
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
        assert!(diagnostic.contains("thinking_chars=17"));
        assert!(diagnostic.contains("done_reason=length"));
    }

    #[test]
    fn detects_separate_and_tagged_thinking_without_treating_it_as_answer_text() {
        let separate = json!({"message":{"content":"","thinking":"internal reasoning"}});
        let tagged = json!({"message":{"content":"<think>internal reasoning</think>"}});
        let ordinary_empty = json!({"message":{"content":""}});
        assert!(has_thinking_output(&separate));
        assert!(has_thinking_output(&tagged));
        assert!(!has_thinking_output(&ordinary_empty));
        assert!(thinking_only_stream_error("Ollama completed without answer text (thinking_chars=18)."));
        assert!(!thinking_only_stream_error("Ollama completed without answer text (thinking_chars=0)."));
    }

    #[test]
    fn detects_thinking_only_when_ollama_advertises_the_capability() {
        let supported = json!({"capabilities":["completion","tools","thinking"]});
        let unsupported = json!({"capabilities":["completion","tools"]});
        assert!(model_supports_thinking(Some(&supported)));
        assert!(!model_supports_thinking(Some(&unsupported)));
        assert!(!model_supports_thinking(None));
    }

    #[test]
    fn prefers_a_bounded_thinking_level_when_ollama_advertises_one() {
        let qwen = json!({"thinking":{"values":["low","medium","high"],"default":"medium"}});
        assert_eq!(model_thinking_mode(Some(&qwen)), Some(json!("low")));
        let boolean_only = json!({"thinking":{"values":[true,false],"default":true}});
        assert_eq!(model_thinking_mode(Some(&boolean_only)), Some(json!(true)));
        let unsupported = json!({"capabilities":["completion","tools"]});
        assert_eq!(model_thinking_mode(Some(&unsupported)), None);
    }

    #[test]
    fn detects_when_a_model_allows_answer_only_fallback() {
        let supported = json!({"thinking":{"values":[true,false],"default":true}});
        let required = json!({"thinking":{"values":[true],"default":true}});
        assert!(model_supports_no_thinking(Some(&supported)));
        assert!(!model_supports_no_thinking(Some(&required)));
    }

    #[test]
    fn identifies_think_option_compatibility_errors() {
        assert!(think_option_error("Ollama returned 400: model does not support thinking"));
        assert!(think_option_error("invalid think parameter"));
        assert!(!think_option_error("model not found"));
    }

    #[test]
    fn thinking_models_receive_a_larger_bounded_output_budget() {
        let mut payload = json!({"options":{"num_predict":180}});
        increase_output_tokens(&mut payload, 2);
        assert_eq!(payload.pointer("/options/num_predict").and_then(Value::as_u64), Some(512));
        payload["options"]["num_predict"] = json!(1800);
        increase_output_tokens(&mut payload, 2);
        assert_eq!(payload.pointer("/options/num_predict").and_then(Value::as_u64), Some(2048));
        payload["options"]["num_predict"] = json!(3000);
        increase_output_tokens(&mut payload, 2);
        assert_eq!(payload.pointer("/options/num_predict").and_then(Value::as_u64), Some(3000));
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
