use crate::adapters::sqlite::knowledge_repo::KnowledgeRepository;
use crate::config::AppConfig;
use crate::error::{HydrogenError, Result};
use serde_json::{json, Value};
use std::io::Read;
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Request, Response, Server, StatusCode};

const FRONTEND: &str = include_str!("../frontend/index.html");

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static HTTP header")
}

fn json_response(value: Value, status: u16, origin: Option<&str>) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{\"error\":\"serialization failure\"}".to_vec());
    let mut response = Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"));
    if let Some(origin) = origin {
        if origin.starts_with("moz-extension://") || origin.starts_with("chrome-extension://")
            || origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:")
        {
            response.add_header(header("Access-Control-Allow-Origin", origin));
            response.add_header(header("Vary", "Origin"));
            response.add_header(header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"));
            response.add_header(header("Access-Control-Allow-Headers", "Content-Type"));
        }
    }
    response
}

fn request_origin(request: &Request) -> Option<String> {
    request.headers().iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str().to_string())
}

fn origin_allowed(origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(value) => value.starts_with("moz-extension://")
            || value.starts_with("chrome-extension://")
            || value.starts_with("http://127.0.0.1:")
            || value.starts_with("http://localhost:"),
    }
}

fn read_json(request: &mut Request) -> std::result::Result<Value, String> {
    let mut body = String::new();
    request.as_reader().take(2 * 1024 * 1024).read_to_string(&mut body)
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&body).map_err(|error| format!("invalid JSON body: {error}"))
}

fn query_param(url: &str, key: &str) -> Option<String> {
    url.split_once('?')?.1.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        if name != key { return None; }
        Some(value.replace('+', " "))
    })
}

fn decode_query(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                output.push(hex);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

fn run_cli_json(args: &[&str]) -> std::result::Result<Value, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let output = Command::new(executable).args(args).output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Hydrogen CLI returned invalid JSON: {error}"))
}

fn handle_request(mut request: Request, cfg: &mut AppConfig) {
    let response = (|| -> Response<std::io::Cursor<Vec<u8>>> {
    let origin = request_origin(&request);
    let origin_ref = origin.as_deref();
    if !origin_allowed(origin_ref) {
        return json_response(json!({"error":"origin is not allowed"}), 403, origin_ref);
    }

    let method = request.method().as_str().to_string();
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or("/");
    if method == "OPTIONS" {
        return json_response(json!({"status":"ok"}), 204, origin_ref);
    }

    if method == "GET" && path == "/" {
        return Response::from_string(FRONTEND)
            .with_header(header("Content-Type", "text/html; charset=utf-8"))
            .with_header(header("Cache-Control", "no-store"));
    }

    if method == "GET" && path == "/api/health" {
        return json_response(json!({
            "status":"ok",
            "model":cfg.settings.answer.model,
            "database":cfg.db_path,
            "version":env!("CARGO_PKG_VERSION")
        }), 200, origin_ref);
    }

    if method == "GET" && path == "/api/settings" {
        return json_response(json!({"settings":cfg.settings}), 200, origin_ref);
    }

    if method == "POST" && path == "/api/settings" {
        let body = match read_json(&mut request) {
            Ok(value) => value,
            Err(error) => return json_response(json!({"error":error}), 400, origin_ref),
        };
        let key = body.get("key").and_then(Value::as_str).unwrap_or("");
        let value = body.get("value").map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        let mut settings = cfg.settings.clone();
        if let Err(error) = crate::cli::config_cmd::set_value(&mut settings, key, &value) {
            return json_response(json!({"error":error.to_string()}), 400, origin_ref);
        }
        if let Err(error) = cfg.write_settings(&settings) {
            return json_response(json!({"error":error}), 500, origin_ref);
        }
        match AppConfig::load() {
            Ok(updated) => *cfg = updated,
            Err(error) => return json_response(json!({"error":error}), 500, origin_ref),
        }
        return json_response(json!({"status":"updated","key":key,"value":value}), 200, origin_ref);
    }

    let knowledge = match KnowledgeRepository::open(&cfg.db_path) {
        Ok(repo) => repo,
        Err(error) => return json_response(json!({"error":error.to_string()}), 500, origin_ref),
    };

    if method == "GET" && path == "/api/conversations" {
        return match knowledge.list_conversations(100) {
            Ok(conversations) => json_response(json!({"conversations":conversations}), 200, origin_ref),
            Err(error) => json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
    }

    if method == "POST" && path == "/api/conversations" {
        if !cfg.settings.conversations.enabled {
            return json_response(json!({"error":"conversation storage is disabled in Hydrogen configuration"}), 403, origin_ref);
        }
        let body = match read_json(&mut request) {
            Ok(value) => value,
            Err(error) => return json_response(json!({"error":error}), 400, origin_ref),
        };
        let title = body.get("title").and_then(Value::as_str).unwrap_or("New conversation");
        return match knowledge.create_conversation(title, &cfg.settings.answer.model) {
            Ok(id) => json_response(json!({"id":id}), 201, origin_ref),
            Err(error) => json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
    }

    if method == "GET" && path.starts_with("/api/conversations/") {
        let id = path.trim_start_matches("/api/conversations/").parse::<i64>();
        let Ok(id) = id else { return json_response(json!({"error":"invalid conversation id"}), 400, origin_ref); };
        return match knowledge.get_conversation(id) {
            Ok(Some(conversation)) => json_response(json!({"conversation":conversation.conversation,"messages":conversation.messages}), 200, origin_ref),
            Ok(None) => json_response(json!({"error":"conversation not found"}), 404, origin_ref),
            Err(error) => json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
    }

    if method == "POST" && path.starts_with("/api/conversations/") && path.ends_with("/messages") {
        let rest = path.trim_start_matches("/api/conversations/").trim_end_matches("/messages").trim_end_matches('/');
        let id = rest.parse::<i64>();
        let Ok(id) = id else { return json_response(json!({"error":"invalid conversation id"}), 400, origin_ref); };
        let body = match read_json(&mut request) {
            Ok(value) => value,
            Err(error) => return json_response(json!({"error":error}), 400, origin_ref),
        };
        let question = body.get("content").and_then(Value::as_str).unwrap_or("").trim();
        if question.is_empty() || question.len() > 32_000 {
            return json_response(json!({"error":"message must contain 1 to 32000 bytes"}), 400, origin_ref);
        }
        let history = match knowledge.get_conversation(id) {
            Ok(Some(conversation)) => conversation.messages.iter()
                .rev()
                .filter(|message| message.role == "user" || message.role == "assistant")
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|message| format!("{}: {}", message.role, message.content))
                .collect::<Vec<_>>()
                .join("\n"),
            Ok(None) => return json_response(json!({"error":"conversation not found"}), 404, origin_ref),
            Err(error) => return json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
        if let Err(error) = knowledge.append_message(id, "user", question, None, None) {
            return json_response(json!({"error":error.to_string()}), 404, origin_ref);
        }
        let generated = run_cli_json(&[
            "answer", question, "--model", cfg.settings.answer.model.as_str(),
            "--history", &history
        ]);
        let (answer, sources, error) = match generated {
            Ok(value) => {
                let answer = value.get("answer").and_then(Value::as_str).unwrap_or("").to_string();
                let sources = value.get("sources").cloned().unwrap_or_else(|| json!([]));
                let error = value.get("error").and_then(Value::as_str).map(str::to_owned)
                    .or_else(|| {
                        let status = value.get("status").and_then(Value::as_str).unwrap_or("");
                        if status == "no_context" {
                            Some("No supporting passages were found in your indexed documents.".into())
                        } else if status != "ready" && answer.trim().is_empty() {
                            Some("The local model did not produce a source-grounded answer.".into())
                        } else { None }
                    });
                (answer, sources, error)
            }
            Err(error) => (String::new(), json!([]), Some(error)),
        };
        let answer_to_save = if answer.trim().is_empty() {
            error.clone().unwrap_or_else(|| "I couldn't generate an answer from the indexed sources.".into())
        } else { answer.clone() };
        let sources_json = serde_json::to_string(&sources).ok();
        if let Err(save_error) = knowledge.append_message(
            id, "assistant", &answer_to_save, Some(&cfg.settings.answer.model), sources_json.as_deref()
        ) {
            return json_response(json!({"error":save_error.to_string()}), 500, origin_ref);
        }
        return json_response(json!({
            "answer":answer,
            "sources":sources,
            "error":error,
            "model":cfg.settings.answer.model
        }), 200, origin_ref);
    }

    if method == "GET" && path == "/api/search" {
        let query = query_param(&url, "q").map(|v| decode_query(&v)).unwrap_or_default();
        if query.trim().is_empty() {
            return json_response(json!({"files":[],"knowledge":[]}), 200, origin_ref);
        }
        let limit = cfg.settings.search.default_limit.min(30).to_string();
        let results = run_cli_json(&["search", &query, "--limit", &limit])
            .unwrap_or_else(|error| json!({"files":[],"knowledge":[],"error":error}));
        return json_response(results, 200, origin_ref);
    }


    if method == "GET" && path.starts_with("/api/web-pages/") {
        let id = path.trim_start_matches("/api/web-pages/").parse::<i64>();
        let Ok(id) = id else { return json_response(json!({"error":"invalid web page id"}), 400, origin_ref); };
        return match knowledge.get_web_page(id) {
            Ok(Some(page)) => json_response(page, 200, origin_ref),
            Ok(None) => json_response(json!({"error":"saved web page not found"}), 404, origin_ref),
            Err(error) => json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
    }

    if method == "POST" && path == "/api/capture/conversation" {
        if !cfg.settings.conversations.enabled || !cfg.settings.conversations.capture_ai_chats {
            return json_response(json!({"error":"AI conversation capture is disabled in Hydrogen configuration"}), 403, origin_ref);
        }
        let body = match read_json(&mut request) {
            Ok(value) => value,
            Err(error) => return json_response(json!({"error":error}), 400, origin_ref),
        };
        let platform = body.get("platform").and_then(Value::as_str).unwrap_or("unknown");
        let url = body.get("url").and_then(Value::as_str).unwrap_or("");
        let title = body.get("title").and_then(Value::as_str).unwrap_or("Imported conversation");
        let raw_messages = body.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
        if url.is_empty() || raw_messages.is_empty() || raw_messages.len() > 500 {
            return json_response(json!({"error":"conversation URL and 1 to 500 messages are required"}), 400, origin_ref);
        }
        let messages: Vec<(String, String)> = raw_messages.iter().filter_map(|message| {
            let role = message.get("role")?.as_str()?.to_string();
            let content = message.get("content")?.as_str()?.to_string();
            Some((role, content))
        }).collect();
        let source_key = format!("{platform}:{url}");
        return match knowledge.import_captured_conversation(&source_key, title, platform, &messages) {
            Ok(Some(id)) => json_response(json!({"status":"saved","conversation_id":id}), 201, origin_ref),
            Ok(None) => json_response(json!({"status":"already_saved"}), 200, origin_ref),
            Err(error) => json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
    }

    if method == "POST" && path == "/api/history/import" {
        let body = match read_json(&mut request) {
            Ok(value) => value,
            Err(error) => return json_response(json!({"error":error}), 400, origin_ref),
        };
        let raw = body.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
        if raw.len() > 1000 {
            return json_response(json!({"error":"history import is limited to 1000 entries per batch"}), 413, origin_ref);
        }
        let entries: Vec<(String,String,i64,i64,i64)> = raw.iter().filter_map(|entry| {
            let url = entry.get("url")?.as_str()?.to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) { return None; }
            let host = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("")
                .split(':').next().unwrap_or("").to_ascii_lowercase();
            if cfg.settings.web.exclude_domains.iter().any(|rule| {
                host == rule.to_ascii_lowercase()
                    || host.ends_with(&format!(".{}", rule.to_ascii_lowercase()))
            }) { return None; }
            let title = entry.get("title").and_then(Value::as_str).unwrap_or("").to_string();
            let last_visit = entry.get("lastVisitTime").and_then(Value::as_i64).unwrap_or(0) / 1000;
            let visits = entry.get("visitCount").and_then(Value::as_i64).unwrap_or(1).max(1);
            let typed = entry.get("typedCount").and_then(Value::as_i64).unwrap_or(0).max(0);
            Some((url,title,last_visit,visits,typed))
        }).collect();
        return match knowledge.import_browser_history(&entries) {
            Ok(imported) => json_response(json!({"status":"imported","entries":imported}), 200, origin_ref),
            Err(error) => json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
    }

    if method == "POST" && path == "/api/feeds" {
        if !cfg.settings.web.enabled || !cfg.settings.web.feeds_enabled {
            return json_response(json!({"error":"RSS and Atom feeds are disabled"}), 403, origin_ref);
        }
        let body = match read_json(&mut request) {
            Ok(value) => value,
            Err(error) => return json_response(json!({"error":error}), 400, origin_ref),
        };
        let url = body.get("url").and_then(Value::as_str).unwrap_or("");
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return json_response(json!({"error":"feed URL must use HTTP or HTTPS"}), 400, origin_ref);
        }
        return match knowledge.add_feed(url) {
            Ok(id) => json_response(json!({"status":"added","id":id}), 201, origin_ref),
            Err(error) => json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
    }

    if method == "POST" && path == "/api/web-pages" {
        if !cfg.settings.web.enabled {
            return json_response(json!({"error":"web knowledge storage is disabled"}), 403, origin_ref);
        }
        let body = match read_json(&mut request) {
            Ok(value) => value,
            Err(error) => return json_response(json!({"error":error}), 400, origin_ref),
        };
        let url = body.get("url").and_then(Value::as_str).unwrap_or("");
        let canonical = body.get("canonicalUrl").and_then(Value::as_str).filter(|v| !v.is_empty()).unwrap_or(url);
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return json_response(json!({"error":"only HTTP(S) pages can be captured"}), 400, origin_ref);
        }
        let title = body.get("title").and_then(Value::as_str).unwrap_or("");
        let text = body.get("text").and_then(Value::as_str).unwrap_or("");
        let html = body.get("html").and_then(Value::as_str).unwrap_or("");
        let manually_saved = body.get("manual").and_then(Value::as_bool).unwrap_or(false);
        if !cfg.settings.web.auto_index && !manually_saved {
            return json_response(json!({"error":"automatic web capture is disabled"}), 403, origin_ref);
        }
        let host = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("")
            .split(':').next().unwrap_or("").to_ascii_lowercase();
        let matches_domain = |rule: &String| host == rule.to_ascii_lowercase()
            || host.ends_with(&format!(".{}", rule.to_ascii_lowercase()));
        if cfg.settings.web.exclude_domains.iter().any(matches_domain) {
            return json_response(json!({"status":"skipped","reason":"domain is excluded by Hydrogen configuration"}), 200, origin_ref);
        }
        if !manually_saved && !cfg.settings.web.allow_domains.is_empty()
            && !cfg.settings.web.allow_domains.iter().any(matches_domain) {
            return json_response(json!({"status":"skipped","reason":"domain is not in the configured allow-list"}), 200, origin_ref);
        }
        if cfg.settings.web.exclude_patterns.iter().any(|pattern| {
            let needle = pattern.replace('*', "");
            !needle.is_empty() && url.contains(&needle)
        }) {
            return json_response(json!({"status":"skipped","reason":"URL matches an excluded pattern"}), 200, origin_ref);
        }
        let paragraphs = text.split("\n\n").filter(|p| p.trim().len() > 40).count();
        if !manually_saved && (text.chars().count() < cfg.settings.web.minimum_text_length || paragraphs < cfg.settings.web.minimum_paragraphs) {
            return json_response(json!({"status":"skipped","reason":"page did not meet the configured article-content threshold"}), 200, origin_ref);
        }
        let max_bytes = cfg.settings.web.max_page_size_mb.saturating_mul(1024 * 1024);
        if text.len() > max_bytes || html.len() > max_bytes {
            return json_response(json!({"error":"captured page exceeds configured size limit"}), 413, origin_ref);
        }
        let author = body.get("author").and_then(Value::as_str).unwrap_or("");
        let published = body.get("publishedAt").and_then(Value::as_str).unwrap_or("");
        let site = body.get("siteName").and_then(Value::as_str).unwrap_or("");
        let digest = blake3::hash(text.as_bytes()).to_hex().to_string();
        return match knowledge.save_web_page(
            url, canonical, title, author, published, site, text,
            if cfg.settings.web.save_html { html } else { "" },
            &digest, manually_saved, cfg.settings.web.track_visits, cfg.settings.web.version_pages
        ) {
            Ok(id) => json_response(json!({"status":"saved","id":id,"content_hash":digest}), 201, origin_ref),
            Err(error) => json_response(json!({"error":error.to_string()}), 500, origin_ref),
        };
    }

    json_response(json!({"error":"not found"}), 404, origin_ref)
    })();
    let _ = request.respond(response);
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

pub fn run(cfg: &AppConfig, ask: Option<&str>, conversation: Option<i64>, page: Option<i64>) -> Result<()> {
    if !cfg.settings.frontend.enabled {
        return Err(HydrogenError::Config("frontend is disabled in Hydrogen configuration".into()));
    }
    let bind = cfg.settings.frontend.bind.as_str();
    if !matches!(bind, "127.0.0.1" | "localhost") {
        return Err(HydrogenError::Config("Hydrogen frontend may only bind to loopback addresses".into()));
    }
    let address = format!("{bind}:{}", cfg.settings.frontend.port);
    let mut url = format!("http://{}:{}/", bind, cfg.settings.frontend.port);
    if let Some(question) = ask.filter(|value| !value.trim().is_empty()) {
        url.push_str("?ask=");
        url.push_str(&encode_query(question.trim()));
    } else if let Some(id) = conversation {
        url.push_str(&format!("?conversation={id}"));
    } else if let Some(id) = page {
        url.push_str(&format!("?page={id}"));
    }

    if TcpStream::connect_timeout(
        &format!("{bind}:{}", cfg.settings.frontend.port).parse().map_err(|error| HydrogenError::Config(format!("invalid frontend address: {error}")))?,
        Duration::from_millis(250)
    ).is_ok() {
        if cfg.settings.frontend.auto_open {
            let _ = Command::new("xdg-open").arg(&url).spawn();
        }
        println!("{}", json!({"status":"already_running","url":url}));
        return Ok(());
    }

    let server = Server::http(&address)
        .map_err(|error| HydrogenError::ExternalTool(format!("could not start Hydrogen frontend on {address}: {error}")))?;
    if cfg.settings.frontend.auto_open {
        let _ = Command::new("xdg-open").arg(&url).spawn();
    }
    println!("{}", json!({"status":"ready","url":url,"database":cfg.db_path}));
    let _feed_worker = thread::spawn(|| loop {
        if let Ok(active) = AppConfig::load() {
            if active.settings.web.enabled && active.settings.web.feeds_enabled {
                let _ = crate::cli::web_cmd::sync_feeds(&active);
            }
            let hours = active.settings.web.feed_check_interval_hours.clamp(1, 168);
            thread::sleep(Duration::from_secs(hours.saturating_mul(3600)));
        } else {
            thread::sleep(Duration::from_secs(3600));
        }
    });
    let mut active_config = cfg.clone();
    for request in server.incoming_requests() {
        handle_request(request, &mut active_config);
    }
    Ok(())
}
