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

const ADDRESS: &str = "127.0.0.1:17891";
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
    respond(request, status, "application/json; charset=utf-8", value.to_string());
}

fn read_body(request: &mut Request) -> std::io::Result<String> {
    let mut body = String::new();
    request.as_reader().take(2 * 1024 * 1024).read_to_string(&mut body)?;
    Ok(body)
}

fn run_cli(args: &[&str]) -> std::io::Result<std::process::Output> {
    let executable = std::env::current_exe()?;
    Command::new(executable).args(args).output()
}

fn handle_api(mut request: Request, cfg: &AppConfig, store: &KnowledgeStore) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let parts: Vec<&str> = url.split('?').next().unwrap_or("/").split('/').filter(|s| !s.is_empty()).collect();

    if method == Method::Get && url.starts_with("/api/health") {
        json(request, 200, json!({"status":"ok","model":cfg.settings.answer.model}));
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
            Err(error) => { json(request, 400, json!({"error":error.to_string()})); return; }
        };
        let input: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let title = input.get("title").and_then(Value::as_str).unwrap_or("New conversation");
        match store.create_conversation(title) {
            Ok(id) => json(request, 201, json!({"id":id,"title":title})),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.len() == 3 && parts[0] == "api" && parts[1] == "conversations" && method == Method::Get {
        let id = parts[2];
        match store.get_messages(id) {
            Ok(messages) => json(request, 200, json!({"messages":messages})),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.len() == 4 && parts[0] == "api" && parts[1] == "conversations"
        && parts[3] == "messages" && method == Method::Post
    {
        let conversation_id = parts[2];
        let body = match read_body(&mut request) {
            Ok(body) => body,
            Err(error) => { json(request, 400, json!({"error":error.to_string()})); return; }
        };
        let input: Value = match serde_json::from_str(&body) {
            Ok(input) => input,
            Err(error) => { json(request, 400, json!({"error":error.to_string()})); return; }
        };
        let content = input.get("content").and_then(Value::as_str).unwrap_or("").trim();
        if content.is_empty() {
            json(request, 400, json!({"error":"message content is required"}));
            return;
        }
        if let Err(error) = store.append_message(conversation_id, "user", content, None) {
            json(request, 404, json!({"error":error.to_string()}));
            return;
        }
        let output = match run_cli(&["answer", content]) {
            Ok(output) => output,
            Err(error) => { json(request, 503, json!({"error":format!("could not start Hydrogen answer worker: {error}")})); return; }
        };
        let result: Value = match serde_json::from_slice(&output.stdout) {
            Ok(value) => value,
            Err(error) => {
                json(request, 503, json!({"error":format!("answer worker returned invalid JSON: {error}"),"stderr":String::from_utf8_lossy(&output.stderr)}));
                return;
            }
        };
        if result.get("status").and_then(Value::as_str) == Some("ready") {
            let answer = result.get("answer").and_then(Value::as_str).unwrap_or("");
            let model = result.get("model").and_then(Value::as_str);
            if let Err(error) = store.append_message(conversation_id, "assistant", answer, model) {
                json(request, 500, json!({"error":error.to_string(),"generation":result}));
                return;
            }
        }
        json(request, 200, result);
        return;
    }

    if parts.as_slice() == ["api", "settings"] && method == Method::Get {
        match run_cli(&["config", "show"]) {
            Ok(output) if output.status.success() => {
                let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| json!({"error":"invalid config response"}));
                json(request, 200, value);
            }
            Ok(output) => json(request, 500, json!({"error":String::from_utf8_lossy(&output.stderr)})),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.as_slice() == ["api", "settings"] && method == Method::Post {
        let body = match read_body(&mut request) {
            Ok(body) => body,
            Err(error) => { json(request, 400, json!({"error":error.to_string()})); return; }
        };
        let input: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let key = input.get("key").and_then(Value::as_str).unwrap_or("");
        let value = input.get("value").map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }).unwrap_or_default();
        if key.is_empty() {
            json(request, 400, json!({"error":"setting key is required"}));
            return;
        }
        match run_cli(&["config", "set", key, &value]) {
            Ok(output) if output.status.success() => {
                let response: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| json!({"status":"updated"}));
                json(request, 200, response);
            }
            Ok(output) => json(request, 400, json!({"error":String::from_utf8_lossy(&output.stderr)})),
            Err(error) => json(request, 500, json!({"error":error.to_string()})),
        }
        return;
    }

    if parts.as_slice() == ["api", "search"] && method == Method::Get {
        let query = url.split_once("?q=").map(|(_, q)| q.replace('+', " ")).unwrap_or_default();
        match store.search_saved_knowledge(&query, 30) {
            Ok(results) => json(request, 200, json!({"results":results})),
            Err(error) => json(request, 400, json!({"error":error.to_string()})),
        }
        return;
    }

    json(request, 404, json!({"error":"not found"}));
}

pub fn run(cfg: &AppConfig) -> Result<()> {
    let server = Server::http(ADDRESS).map_err(|error| {
        HydrogenError::Config(format!("could not bind Hydrogen frontend at {ADDRESS}: {error}"))
    })?;
    let store = KnowledgeStore::open(&cfg.db_path)
        .map_err(|error| HydrogenError::Config(format!("could not open Hydrogen knowledge store: {error}")))?;
    eprintln!("Hydrogen frontend API listening at http://{ADDRESS}");
    for request in server.incoming_requests() {
        let url = request.url().to_string();
        if request.method() == &Method::Get && (url == "/" || url.starts_with("/?")) {
            respond(request, 200, "text/html; charset=utf-8", FRONTEND.to_string());
        } else if url.starts_with("/api/") {
            handle_api(request, cfg, &store);
        } else {
            respond(request, 404, "text/plain; charset=utf-8", "Not found".into());
        }
    }
    Ok(())
}

pub fn open_frontend() -> Result<()> {
    if TcpStream::connect(ADDRESS).is_err() {
        let executable = std::env::current_exe()?;
        Command::new(executable)
            .arg("serve")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| HydrogenError::Config(format!("could not start Hydrogen frontend service: {error}")))?;
        let mut ready = false;
        for _ in 0..40 {
            thread::sleep(Duration::from_millis(100));
            if TcpStream::connect(ADDRESS).is_ok() {
                ready = true;
                break;
            }
        }
        if !ready {
            return Err(HydrogenError::Config("Hydrogen frontend service did not become ready on 127.0.0.1:17891".into()));
        }
    }
    Command::new("xdg-open")
        .arg(format!("http://{ADDRESS}/"))
        .spawn()
        .map_err(|error| HydrogenError::Config(format!("could not open Hydrogen in the browser: {error}")))?;
    Ok(())
}
