use crate::adapters::sqlite::knowledge::KnowledgeStore;
use crate::config::AppConfig;
use crate::error::{HydrogenError, Result};
use serde_json::{json, Value};
use std::io::Read;
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const FRONTEND: &str = include_str!("../frontend/index.html");

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid static HTTP header")
}

fn respond(request: Request, status: u16, content_type: &str, body: String) {
    let response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"));
    let _ = request.respond(response);
}

fn json(request: Request, status: u16, value: Value) {
    respond(
        request,
        status,
        "application/json; charset=utf-8",
        value.to_string(),
    );
}

fn read_body(request: &mut Request) -> std::io::Result<String> {
    let mut body = String::new();
    request
        .as_reader()
        .take(2 * 1024 * 1024)
        .read_to_string(&mut body)?;
    Ok(body)
}

fn decode_query_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(byte);
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn run_cli(args: &[&str]) -> std::io::Result<std::process::Output> {
    let executable = std::env::current_exe()?;
    Command::new(executable).args(args).output()
}

fn handle_api(mut request: Request, cfg: &AppConfig, store: &KnowledgeStore) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let parts: Vec<&str> = url
        .split('?')
        .next()
        .unwrap_or("/")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    if method == Method::Get && url.starts_with("/api/health") {
        json(
            request,
            200,
            json!({"status":"ok","model":cfg.settings.answer.model}),
        );
        return;
    }

    if parts.as_slice() == ["api", "conversations"] && method == Method::Get {
        match store.list_conversations(100) {
            Ok(items) => json(request, 200, json!({"conversations":items})),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.as_slice() == ["api", "conversations"] && method == Method::Post {
        let body = match read_body(&mut request) {
            Ok(body) => body,
            Err(error) => {
                json(request, 400, json!({"error":error.to_string()}));
                return;
            }
        };
        let input: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let title = input
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("New conversation");
        match store.create_conversation(title) {
            Ok(id) => json(request, 201, json!({"id":id,"title":title})),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.len() == 3 && parts[0] == "api" && parts[1] == "conversations" && method == Method::Get
    {
        let id = parts[2];
        match store.get_messages(id) {
            Ok(messages) => json(request, 200, json!({"messages":messages})),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.len() == 4
        && parts[0] == "api"
        && parts[1] == "conversations"
        && parts[3] == "messages"
        && method == Method::Post
    {
        let conversation_id = parts[2];
        let body = match read_body(&mut request) {
            Ok(body) => body,
            Err(error) => {
                json(request, 400, json!({"error":error.to_string()}));
                return;
            }
        };
        let input: Value = match serde_json::from_str(&body) {
            Ok(input) => input,
            Err(error) => {
                json(request, 400, json!({"error":error.to_string()}));
                return;
            }
        };
        let content = input
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if content.is_empty() {
            json(request, 400, json!({"error":"message content is required"}));
            return;
        }
        if let Err(error) = store.append_message(conversation_id, "user", content, None) {
            json(request, 404, json!({"error":error.to_string()}));
            return;
        }
        let output = match run_cli(&["answer", "--no-save", content]) {
            Ok(output) => output,
            Err(error) => {
                json(
                    request,
                    503,
                    json!({"error":format!("could not start Hydrogen answer worker: {error}")}),
                );
                return;
            }
        };
        let result: Value = match serde_json::from_slice(&output.stdout) {
            Ok(value) => value,
            Err(error) => {
                json(
                    request,
                    503,
                    json!({"error":format!("answer worker returned invalid JSON: {error}"),"stderr":String::from_utf8_lossy(&output.stderr)}),
                );
                return;
            }
        };
        if result.get("status").and_then(Value::as_str) == Some("ready") {
            let answer = result.get("answer").and_then(Value::as_str).unwrap_or("");
            let model = result.get("model").and_then(Value::as_str);
            let sources = result.get("sources");
            if let Err(error) = store.append_message_with_sources(
                conversation_id,
                "assistant",
                answer,
                model,
                sources,
            ) {
                json(
                    request,
                    500,
                    json!({"error":error.to_string(),"generation":result}),
                );
                return;
            }
        }
        json(request, 200, result);
        return;
    }

    if parts.len() == 3 && parts[0] == "api" && parts[1] == "web-pages" && method == Method::Get {
        match parts[2].parse::<i64>() {
            Ok(id) => match store.get_web_page(id) {
                Ok(Some(page)) => {
                    if cfg.settings.web.track_visits {
                        let _ = store.record_visit(&page.canonical_url);
                    }
                    json(request, 200, json!({"page":page}));
                }
                Ok(None) => json(request, 404, json!({"error":"saved page not found"})),
                Err(error) => json(request, 500, json!({"error":error.to_string()})),
            },
            Err(_) => json(request, 400, json!({"error":"invalid saved page id"})),
        }
        return;
    }

    if parts.as_slice() == ["api", "web-pages"] && method == Method::Post {
        let body = match read_body(&mut request) {
            Ok(body) => body,
            Err(error) => {
                json(request, 400, json!({"error":error.to_string()}));
                return;
            }
        };
        let input: Value = match serde_json::from_str(&body) {
            Ok(input) => input,
            Err(error) => {
                json(request, 400, json!({"error":error.to_string()}));
                return;
            }
        };
        let url = input.get("url").and_then(Value::as_str).unwrap_or("");
        let title = input.get("title").and_then(Value::as_str).unwrap_or("");
        let content = input.get("content").and_then(Value::as_str).unwrap_or("");
        let canonical = input
            .get("canonical_url")
            .and_then(Value::as_str)
            .unwrap_or(url);
        let manual = input
            .get("manual")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !cfg.settings.web.enabled || (!cfg.settings.web.auto_index && !manual) {
            json(
                request,
                403,
                json!({"error":"automatic web indexing is disabled in Hydrogen configuration"}),
            );
            return;
        }
        if url.is_empty() || title.is_empty() || content.trim().is_empty() {
            json(
                request,
                400,
                json!({"error":"url, title and readable content are required"}),
            );
            return;
        }
        let host = url
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(&url)
            .split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let domain_matches = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
        if cfg
            .settings
            .web
            .exclude_domains
            .iter()
            .any(|domain| domain_matches(&domain.to_ascii_lowercase()))
        {
            json(
                request,
                403,
                json!({"error":"this domain is excluded by Hydrogen configuration"}),
            );
            return;
        }
        if !cfg.settings.web.allow_domains.is_empty()
            && !cfg
                .settings
                .web
                .allow_domains
                .iter()
                .any(|domain| domain_matches(&domain.to_ascii_lowercase()))
        {
            json(
                request,
                403,
                json!({"error":"this domain is not in web.allow_domains"}),
            );
            return;
        }
        let excluded_url = cfg.settings.web.exclude_patterns.iter().any(|pattern| {
            pattern
                .split('*')
                .filter(|part| !part.is_empty())
                .all(|part| url.contains(part))
        });
        if excluded_url {
            json(
                request,
                403,
                json!({"error":"this URL matches a Hydrogen exclusion rule"}),
            );
            return;
        }
        let max_bytes = cfg
            .settings
            .web
            .max_page_size_mb
            .saturating_mul(1024 * 1024);
        let text_length = content.chars().count();
        let paragraphs = input
            .get("paragraph_count")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if content.len() > max_bytes
            || (!manual
                && (text_length < cfg.settings.web.minimum_text_length
                    || paragraphs < cfg.settings.web.minimum_paragraphs))
        {
            json(
                request,
                422,
                json!({"error":"page did not meet Hydrogen's readable-content qualification limits","text_length":text_length,"paragraphs":paragraphs}),
            );
            return;
        }
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        match store.save_web_page(
            url,
            canonical,
            title,
            content,
            input.get("author").and_then(Value::as_str),
            input.get("published_at").and_then(Value::as_str),
            input.get("site_name").and_then(Value::as_str),
            &hash,
            cfg.settings.web.version_pages,
        ) {
            Ok(id) => {
                if cfg.settings.web.track_visits {
                    let _ = store.record_visit(canonical);
                }
                json(
                    request,
                    201,
                    json!({"status":"saved","id":id,"canonical_url":canonical,"content_hash":hash}),
                );
            }
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.as_slice() == ["api", "capture", "conversation"] && method == Method::Post {
        let body = match read_body(&mut request) {
            Ok(body) => body,
            Err(error) => {
                json(request, 400, json!({"error":error.to_string()}));
                return;
            }
        };
        let input: Value = match serde_json::from_str(&body) {
            Ok(input) => input,
            Err(error) => {
                json(request, 400, json!({"error":error.to_string()}));
                return;
            }
        };
        let source_key = input
            .get("source_key")
            .and_then(Value::as_str)
            .unwrap_or("");
        let title = input
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Captured conversation");
        let messages: Vec<(String, String, Option<String>)> = input
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|message| {
                let role = message.get("role")?.as_str()?.to_string();
                let content = message.get("content")?.as_str()?.to_string();
                let model = message
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Some((role, content, model))
            })
            .collect();
        if !cfg.settings.conversations.enabled || !cfg.settings.conversations.capture_ai_chats {
            json(
                request,
                403,
                json!({"error":"AI conversation capture is disabled in Hydrogen configuration"}),
            );
            return;
        }
        let host = source_key
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(source_key)
            .split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if cfg.settings.web.exclude_domains.iter().any(|domain| {
            host == domain.to_ascii_lowercase()
                || host.ends_with(&format!(".{}", domain.to_ascii_lowercase()))
        }) {
            json(
                request,
                403,
                json!({"error":"this domain is excluded by Hydrogen configuration"}),
            );
            return;
        }
        match store.capture_conversation(source_key, title, &messages) {
            Ok(id) => json(
                request,
                200,
                json!({"status":"captured","conversation_id":id,"message_count":messages.len()}),
            ),
            Err(error) => json(request, 400, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.as_slice() == ["api", "settings"] && method == Method::Get {
        match run_cli(&["config", "show"]) {
            Ok(output) if output.status.success() => {
                let value: Value = serde_json::from_slice(&output.stdout)
                    .unwrap_or_else(|_| json!({"error":"invalid config response"}));
                json(request, 200, value);
            }
            Ok(output) => json(
                request,
                500,
                json!({"error":String::from_utf8_lossy(&output.stderr)}),
            ),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.as_slice() == ["api", "settings"] && method == Method::Post {
        let body = match read_body(&mut request) {
            Ok(body) => body,
            Err(error) => {
                json(request, 400, json!({"error":error.to_string()}));
                return;
            }
        };
        let input: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let key = input.get("key").and_then(Value::as_str).unwrap_or("");
        let value = input
            .get("value")
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default();
        if key.is_empty() {
            json(request, 400, json!({"error":"setting key is required"}));
            return;
        }
        match run_cli(&["config", "set", key, &value]) {
            Ok(output) if output.status.success() => {
                let response: Value = serde_json::from_slice(&output.stdout)
                    .unwrap_or_else(|_| json!({"status":"updated"}));
                json(request, 200, response);
            }
            Ok(output) => json(
                request,
                400,
                json!({"error":String::from_utf8_lossy(&output.stderr)}),
            ),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.as_slice() == ["api", "search"] && method == Method::Get {
        let query = url
            .split_once("?q=")
            .map(|(_, q)| q.replace('+', " "))
            .unwrap_or_default();
        match store.search_saved_knowledge(&query, 30) {
            Ok(saved) => {
                let mut results: Vec<Value> = saved
                    .into_iter()
                    .map(|hit| serde_json::to_value(hit).unwrap_or_else(|_| json!({})))
                    .collect();
                if let Ok(output) = run_cli(&["query", &query]) {
                    if output.status.success() {
                        if let Ok(files) = serde_json::from_slice::<Vec<Value>>(&output.stdout) {
                            for file in files {
                                let path = file.get("path").and_then(Value::as_str).unwrap_or("");
                                results.push(json!({
                                    "kind":"file",
                                    "id":path,
                                    "title":file.get("name").and_then(Value::as_str).unwrap_or(path),
                                    "snippet":file.get("snippet").and_then(Value::as_str).unwrap_or(""),
                                    "path":path,
                                    "page":file.get("page"),
                                    "updated_at":0
                                }));
                            }
                        }
                    }
                }
                json(request, 200, json!({"results":results}));
            }
            Err(error) => json(request, 400, json!({"error":error.to_string()})),
        }
        return;
    }

    json(request, 404, json!({"error":"not found"}));
}

pub fn run(cfg: &AppConfig) -> Result<()> {
    let address = cfg.settings.frontend.bind_address.as_str();
    let server = Server::http(address).map_err(|error| {
        HydrogenError::Config(format!(
            "could not bind Hydrogen frontend at {address}: {error}"
        ))
    })?;
    let store = KnowledgeStore::open(&cfg.db_path).map_err(|error| {
        HydrogenError::Config(format!("could not open Hydrogen knowledge store: {error}"))
    })?;
    eprintln!("Hydrogen frontend API listening at http://{address}");
    for request in server.incoming_requests() {
        let url = request.url().to_string();
        if request.method() == &Method::Get && (url == "/" || url.starts_with("/?")) {
            respond(
                request,
                200,
                "text/html; charset=utf-8",
                FRONTEND.to_string(),
            );
        } else if url.starts_with("/api/") {
            // Reload the persisted configuration between requests so changes made
            // through the settings API affect capture and answer behaviour without
            // requiring a frontend restart. The listening socket itself is restart-only.
            let effective_cfg = AppConfig::load().unwrap_or_else(|_| cfg.clone());
            handle_api(request, &effective_cfg, &store);
        } else {
            respond(
                request,
                404,
                "text/plain; charset=utf-8",
                "Not found".into(),
            );
        }
    }
    Ok(())
}

fn encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub fn open_frontend(cfg: &AppConfig, ask: Option<&str>) -> Result<()> {
    let address = cfg.settings.frontend.bind_address.as_str();
    if TcpStream::connect(address).is_err() {
        if !cfg.settings.frontend.auto_start {
            return Err(HydrogenError::Config(format!(
                "Hydrogen frontend is not running at {address} and frontend.auto_start is disabled"
            )));
        }
        let executable = std::env::current_exe()?;
        Command::new(executable)
            .arg("serve")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| {
                HydrogenError::Config(format!(
                    "could not start Hydrogen frontend service: {error}"
                ))
            })?;
        let mut ready = false;
        for _ in 0..40 {
            thread::sleep(Duration::from_millis(100));
            if TcpStream::connect(address).is_ok() {
                ready = true;
                break;
            }
        }
        if !ready {
            return Err(HydrogenError::Config(format!(
                "Hydrogen frontend service did not become ready at {address}"
            )));
        }
    }
    let url = ask
        .map(|question| format!("http://{address}/?ask={}", encode_query(question)))
        .unwrap_or_else(|| format!("http://{address}/"));
    Command::new("xdg-open").arg(url).spawn().map_err(|error| {
        HydrogenError::Config(format!("could not open Hydrogen in the browser: {error}"))
    })?;
    Ok(())
}
