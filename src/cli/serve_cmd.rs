use crate::adapters::sqlite::knowledge::KnowledgeStore;
use crate::config::AppConfig;
use crate::error::{HydrogenError, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::net::TcpStream;
use std::process::Command;
use std::sync::{Arc, Mutex};
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
        .take(12 * 1024 * 1024)
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
                let hex = |byte: u8| match byte {
                    b'0'..=b'9' => Some(byte - b'0'),
                    b'a'..=b'f' => Some(byte - b'a' + 10),
                    b'A'..=b'F' => Some(byte - b'A' + 10),
                    _ => None,
                };
                if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                    decoded.push((high << 4) | low);
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

fn query_parameter(url: &str, name: &str) -> Option<String> {
    url.split_once('?')?
        .1
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(key, value)| {
            (decode_query_component(key) == name).then(|| decode_query_component(value))
        })
}

fn run_cli(args: &[&str]) -> std::io::Result<std::process::Output> {
    let executable = std::env::current_exe()?;
    Command::new(executable).args(args).output()
}

type JobStore = Arc<Mutex<HashMap<String, Value>>>;

fn set_job(jobs: &JobStore, id: &str, value: Value) {
    if let Ok(mut jobs) = jobs.lock() {
        jobs.insert(id.to_string(), value);
    }
}

fn request_security_error(request: &Request, bind_address: &str) -> Option<(u16, &'static str)> {
    let header_value = |name: &str| {
        request.headers().iter()
            .find(|item| item.field.equiv(name))
            .map(|item| item.value.as_str())
    };

    let port = bind_address.rsplit_once(':').map(|(_, port)| port).unwrap_or("");
    let host = header_value("Host");
    let valid_host = host.map(|value| {
        let value = value.trim().to_ascii_lowercase();
        value == bind_address.to_ascii_lowercase()
            || value == format!("localhost:{port}")
            || value == format!("127.0.0.1:{port}")
            || value == format!("[::1]:{port}")
    }).unwrap_or(false);
    if !valid_host {
        return Some((400, "invalid Host header"));
    }

    if let Some(origin) = header_value("Origin") {
        let origin = origin.trim().trim_end_matches('/');
        let valid_origin = origin == format!("http://localhost:{port}")
            || origin == format!("http://127.0.0.1:{port}")
            || origin == format!("http://[::1]:{port}")
            || origin.starts_with("chrome-extension://");
        if !valid_origin {
            return Some((403, "request Origin is not allowed"));
        }
    }

    let is_mutating = request.method() == &Method::Post
        || request.method() == &Method::Put
        || request.method() == &Method::Patch
        || request.method() == &Method::Delete;
    if is_mutating {
        let content_type = header_value("Content-Type").unwrap_or("");
        if !content_type.to_ascii_lowercase().starts_with("application/json") {
            return Some((415, "mutating API requests must use application/json"));
        }
    }
    None
}

fn handle_api(mut request: Request, cfg: &AppConfig, store: &KnowledgeStore, jobs: &JobStore) {
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

    if parts.as_slice() == ["api", "history"] && method == Method::Get {
        let limit = query_parameter(&url, "limit")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(100)
            .clamp(1, 500);
        match store.list_visits(limit) {
            Ok(visits) => json(request, 200, json!({"visits":visits})),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.as_slice() == ["api", "models"] && method == Method::Get {
        let endpoint = cfg.settings.answer.endpoint.trim_end_matches('/');
        let output = Command::new("curl")
            .args(["--silent", "--show-error", "--fail", "--connect-timeout", "2", "--max-time", "5", &format!("{endpoint}/api/tags")])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                match serde_json::from_slice::<Value>(&output.stdout) {
                    Ok(value) => {
                        let models = value.get("models").and_then(Value::as_array)
                            .cloned().unwrap_or_default();
                        json(request, 200, json!({"models":models,"selected":cfg.settings.answer.model}));
                    }
                    Err(error) => json(request, 502, json!({"error":format!("Ollama returned invalid model data: {error}")})),
                }
            }
            Ok(output) => json(request, 502, json!({
                "error":String::from_utf8_lossy(&output.stderr).trim().to_string(),
                "selected":cfg.settings.answer.model
            })),
            Err(error) => json(request, 502, json!({"error":format!("could not query Ollama models: {error}")})),
        }
        return;
    }

    if parts.len() == 3 && parts[0] == "api" && parts[1] == "jobs" && method == Method::Get {
        let id = parts[2];
        let value = jobs.lock().ok().and_then(|jobs| jobs.get(id).cloned());
        match value {
            Some(value) => json(request, 200, value),
            None => json(request, 404, json!({"error":"generation job not found"})),
        }
        return;
    }

    if parts.as_slice() == ["api", "conversations"] && method == Method::Get {
        let limit = query_parameter(&url, "limit")
            .and_then(|value| value.parse::<usize>().ok()).unwrap_or(100).clamp(1, 500);
        match store.list_conversations(limit) {
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

    if parts.len() == 3
        && parts[0] == "api"
        && parts[1] == "conversations"
        && (method == Method::Patch || method == Method::Delete)
    {
        let id = parts[2];
        if method == Method::Delete {
            match store.delete_conversation(id) {
                Ok(true) => json(request, 200, json!({"status":"deleted","id":id})),
                Ok(false) => json(request, 404, json!({"error":"conversation not found"})),
                Err(error) => json(request, 500, json!({"error":error.to_string()})),
            }
            return;
        }

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
        let title = input.get("title").and_then(Value::as_str);
        let pinned = input.get("pinned").and_then(Value::as_bool);
        if title.is_none() && pinned.is_none() {
            json(request, 400, json!({"error":"provide a title or pinned value"}));
            return;
        }
        if title.map(|value| value.trim().is_empty() || value.chars().count() > 160).unwrap_or(false) {
            json(request, 400, json!({"error":"title must contain 1 to 160 characters"}));
            return;
        }
        match store.update_conversation(id, title, pinned) {
            Ok(true) => json(request, 200, json!({"status":"updated","id":id})),
            Ok(false) => json(request, 404, json!({"error":"conversation not found"})),
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
        let question = content.to_string();
        let selected_model = input.get("model").and_then(Value::as_str)
            .map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned);
        let conversation = conversation_id.to_string();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let material = format!("{}:{}:{}", std::process::id(), stamp, conversation);
        let job_id = format!("job_{}", &blake3::hash(material.as_bytes()).to_hex()[..24]);
        set_job(jobs, &job_id, json!({"id":job_id,"status":"queued"}));

        let worker_jobs = Arc::clone(jobs);
        let worker_conversation = conversation.clone();
        let db_path = cfg.db_path.clone();
        let model = selected_model.unwrap_or_else(|| cfg.settings.answer.model.clone());
        let worker_id = job_id.clone();
        thread::spawn(move || {
            set_job(
                &worker_jobs,
                &worker_id,
                json!({"id":worker_id,"status":"generating","model":model}),
            );
            let output = run_cli(&["answer", "--no-save", "--model", &model, &question]);
            let result = match output {
                Ok(output) => match serde_json::from_slice::<Value>(&output.stdout) {
                    Ok(value) => value,
                    Err(error) => json!({
                        "status":"unavailable",
                        "answer":"",
                        "sources":[],
                        "error":format!(
                            "answer worker returned invalid JSON: {error}; stderr: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        )
                    }),
                },
                Err(error) => json!({
                    "status":"unavailable",
                    "answer":"",
                    "sources":[],
                    "error":format!("could not start Hydrogen answer worker: {error}")
                }),
            };

            if result.get("status").and_then(Value::as_str) == Some("ready") {
                let answer = result.get("answer").and_then(Value::as_str).unwrap_or("");
                let model = result.get("model").and_then(Value::as_str);
                let sources = result.get("sources");
                let saved = KnowledgeStore::open(&db_path).and_then(|knowledge| {
                    knowledge.append_message_with_sources(
                        &worker_conversation,
                        "assistant",
                        answer,
                        model,
                        sources,
                    )
                });
                if let Err(error) = saved {
                    set_job(
                        &worker_jobs,
                        &worker_id,
                        json!({
                            "id":worker_id,
                            "status":"failed",
                            "error":format!("answer generated but could not be saved: {error}"),
                            "generation":result
                        }),
                    );
                    return;
                }
            }

            let status = result
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unavailable");
            let final_status = if status == "ready" {
                "ready"
            } else {
                "unavailable"
            };
            set_job(
                &worker_jobs,
                &worker_id,
                json!({
                    "id":worker_id,
                    "status":final_status,
                    "result":result
                }),
            );
        });

        json(
            request,
            202,
            json!({"status":"queued","job_id":job_id,"conversation_id":conversation}),
        );
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
        let query = query_parameter(&url, "q").unwrap_or_default();
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
    let store = Arc::new(KnowledgeStore::open(&cfg.db_path).map_err(|error| {
        HydrogenError::Config(format!("could not open Hydrogen knowledge store: {error}"))
    })?);
    let jobs: JobStore = Arc::new(Mutex::new(HashMap::new()));
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
            if let Some((status, message)) = request_security_error(&request, address) {
                json(request, status, json!({"error":message}));
                continue;
            }
            // Each request gets its own worker thread. Long-running RAG inference
            // must never block health checks, history, search, or job polling.
            let effective_cfg = AppConfig::load().unwrap_or_else(|_| cfg.clone());
            let request_store = Arc::clone(&store);
            let request_jobs = Arc::clone(&jobs);
            thread::spawn(move || {
                handle_api(request, &effective_cfg, &request_store, &request_jobs);
            });
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

pub fn open_frontend(
    cfg: &AppConfig,
    ask: Option<&str>,
    conversation: Option<&str>,
    page: Option<&str>,
) -> Result<()> {
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
    let base = format!("http://{address}/");
    let url = if let Some(question) = ask {
        format!("{base}?ask={}", encode_query(question))
    } else if let Some(id) = conversation {
        format!("{base}?conversation={}", encode_query(id))
    } else if let Some(id) = page {
        format!("{base}?page={}", encode_query(id))
    } else {
        base
    };
    Command::new("xdg-open").arg(url).spawn().map_err(|error| {
        HydrogenError::Config(format!("could not open Hydrogen in the browser: {error}"))
    })?;
    Ok(())
}

#[cfg(test)]
mod query_tests {
    use super::{decode_query_component, query_parameter};

    #[test]
    fn decodes_spaces_unicode_and_reserved_query_characters() {
        assert_eq!(
            decode_query_component("throttling+device%3F+%E2%9C%93"),
            "throttling device? ✓"
        );
    }

    #[test]
    fn finds_query_parameter_without_consuming_following_parameters() {
        assert_eq!(
            query_parameter("/api/search?kind=all&q=throttling%20device&limit=10", "q").as_deref(),
            Some("throttling device")
        );
    }

    #[test]
    fn missing_query_parameter_is_none() {
        assert_eq!(query_parameter("/api/search?limit=10", "q"), None);
    }
}
