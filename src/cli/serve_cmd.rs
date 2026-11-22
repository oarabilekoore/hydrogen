use crate::adapters::sqlite::knowledge::KnowledgeStore;
use crate::config::AppConfig;
use crate::error::{HydrogenError, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use glob::{MatchOptions, Pattern};
use url::Url;

const FRONTEND: &str = include_str!("../frontend/index.html");

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid static HTTP header")
}

fn respond(request: Request, status: u16, content_type: &str, body: String) {
    let origin = request.headers().iter()
        .find(|item| item.field.equiv("Origin"))
        .map(|item| item.value.as_str().to_string());
    let mut response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"));
    if let Some(origin) = origin.filter(|value| value.starts_with("chrome-extension://")
        || value.starts_with("http://localhost:")
        || value.starts_with("http://127.0.0.1:")
        || value.starts_with("http://[::1]:")) {
        response = response
            .with_header(header("Access-Control-Allow-Origin", &origin))
            .with_header(header("Access-Control-Allow-Methods", "GET, POST, PATCH, DELETE, OPTIONS"))
            .with_header(header("Access-Control-Allow-Headers", "Content-Type, Authorization"))
            .with_header(header("Vary", "Origin"));
    }
    let _ = request.respond(response);
}

fn api_token(cfg: &AppConfig) -> std::io::Result<String> {
    fs::create_dir_all(&cfg.state_dir)?;
    let path = cfg.state_dir.join("hydrogen-api-token");
    if path.exists() {
        let token = fs::read_to_string(path)?.trim().to_string();
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "stored API token is invalid; remove hydrogen-api-token and restart Hydrogen"));
        }
        return Ok(token);
    }
    let mut bytes = [0u8; 32];
    OpenOptions::new().read(true).open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut file = OpenOptions::new().write(true).create_new(true).open(&path)?;
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

pub fn print_api_token(cfg: &AppConfig) -> Result<()> {
    let token = api_token(cfg).map_err(|error| HydrogenError::Config(format!("could not initialise API token: {error}")))?;
    println!("{token}");
    Ok(())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let a = left.as_bytes();
    let b = right.as_bytes();
    let mut difference = a.len() ^ b.len();
    for index in 0..a.len().max(b.len()) {
        difference |= (a.get(index).copied().unwrap_or(0) ^ b.get(index).copied().unwrap_or(0)) as usize;
    }
    difference == 0
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

const JOB_MAX_CHUNKS: usize = 512;
const JOB_MAX_RETAINED: usize = 64;
const JOB_TTL_SECONDS: u64 = 30 * 60;

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn terminal_job(value: &Value) -> bool {
    matches!(
        value.get("status").and_then(Value::as_str),
        Some("ready" | "unavailable" | "failed" | "cancelled")
    )
}

fn prune_jobs(jobs: &mut HashMap<String, Value>) {
    let now = unix_seconds();
    jobs.retain(|_, job| {
        !terminal_job(job)
            || now.saturating_sub(job.get("created_at").and_then(Value::as_u64).unwrap_or(now))
                < JOB_TTL_SECONDS
    });

    if jobs.len() > JOB_MAX_RETAINED {
        let mut completed: Vec<(String, u64)> = jobs
            .iter()
            .filter(|(_, job)| terminal_job(job))
            .map(|(id, job)| (
                id.clone(),
                job.get("created_at").and_then(Value::as_u64).unwrap_or(0),
            ))
            .collect();
        completed.sort_by_key(|(_, created_at)| *created_at);
        let remove_count = completed.len().saturating_sub(JOB_MAX_RETAINED);
        for (id, _) in completed.into_iter().take(remove_count) {
            jobs.remove(&id);
        }
    }
}

fn set_job(jobs: &JobStore, id: &str, mut value: Value) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(previous) = jobs.get(id) {
            for key in ["answer", "chunks", "seq", "created_at", "thinking_chars"] {
                if value.get(key).is_none() {
                    if let Some(previous_value) = previous.get(key) {
                        value[key] = previous_value.clone();
                    }
                }
            }
        }
        value["updated_at"] = json!(unix_seconds());
        jobs.insert(id.to_string(), value);
    }
}

fn append_job_delta(jobs: &JobStore, id: &str, delta: &str) {
    if delta.is_empty() { return; }
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            let sequence = job.get("seq").and_then(Value::as_u64).unwrap_or(0) + 1;
            let mut answer = job.get("answer").and_then(Value::as_str).unwrap_or("").to_string();
            answer.push_str(delta);
            job["answer"] = Value::String(answer);
            job["seq"] = json!(sequence);
            job["status"] = Value::String("streaming".to_string());
            job["updated_at"] = json!(unix_seconds());
            if let Some(chunks) = job["chunks"].as_array_mut() {
                chunks.push(json!({"seq":sequence,"text":delta}));
                if chunks.len() > JOB_MAX_CHUNKS {
                    let excess = chunks.len() - JOB_MAX_CHUNKS;
                    chunks.drain(0..excess);
                }
            }
        }
    }
}

fn append_job_thinking(jobs: &JobStore, id: &str, chars: usize) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            job["thinking_chars"] = json!(chars);
            if job.get("status").and_then(Value::as_str) == Some("generating") {
                job["status"] = Value::String("thinking".into());
            }
            job["updated_at"] = json!(unix_seconds());
        }
    }
}

fn job_snapshot(jobs: &JobStore, id: &str, after: u64) -> Option<Value> {
    let jobs = jobs.lock().ok()?;
    let stored = jobs.get(id)?;
    let mut snapshot = stored.clone();
    let chunks = stored.get("chunks").and_then(Value::as_array).cloned().unwrap_or_default();
    let sequence = stored.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let earliest = chunks.first().and_then(|chunk| chunk.get("seq")).and_then(Value::as_u64).unwrap_or(sequence.saturating_add(1));
    let reset = after.saturating_add(1) < earliest && after < sequence;
    let delta = if reset {
        String::new()
    } else {
        chunks.iter()
            .filter(|chunk| chunk.get("seq").and_then(Value::as_u64).unwrap_or(0) > after)
            .filter_map(|chunk| chunk.get("text").and_then(Value::as_str))
            .collect::<String>()
    };
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("chunks");
        object.insert("seq".into(), json!(sequence));
        object.insert("reset".into(), json!(reset));
        object.insert("delta".into(), json!(delta));
        if !reset && !terminal_job(stored) {
            object.remove("answer");
        }
    }
    Some(snapshot)
}

fn queue_answer_job(jobs: &JobStore, conversation_id: &str, question: String, model: String, grounding: bool, db_path: std::path::PathBuf) -> String {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let material = format!("{}:{}:{}", std::process::id(), stamp, conversation_id);
    let job_id = format!("job_{}", &blake3::hash(material.as_bytes()).to_hex()[..24]);
    if let Ok(mut jobs) = jobs.lock() {
        prune_jobs(&mut jobs);
        jobs.insert(job_id.clone(), json!({"id":job_id,"status":"queued","answer":"","chunks":[],"seq":0,"thinking_chars":0,"created_at":unix_seconds(),"updated_at":unix_seconds(),"model":model}));
    }
    let worker_jobs = Arc::clone(jobs);
    let worker_conversation = conversation_id.to_string();
    let worker_id = job_id.clone();
    thread::spawn(move || {
        set_job(&worker_jobs, &worker_id, json!({"id":worker_id,"status":"generating","answer":"","model":model}));
        let executable = match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":"failed","error":format!("could not locate Hydrogen executable: {error}")}));
                return;
            }
        };
        let mut worker = Command::new(executable);
        worker.args(["answer","--no-save","--stream","--conversation",&worker_conversation,"--model",&model]);
        if !grounding { worker.arg("--no-grounding"); }
        let mut child = match worker.arg(&question)
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
            Ok(child) => child,
            Err(error) => {
                set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":"failed","error":format!("could not start Hydrogen answer worker: {error}")}));
                return;
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":"failed","error":"answer worker stdout was unavailable"}));
                return;
            }
        };
        let reader = BufReader::new(stdout);
        let mut result: Option<Value> = None;
        let mut parse_error: Option<String> = None;
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => { parse_error = Some(format!("could not read answer stream: {error}")); break; }
            };
            if line.trim().is_empty() { continue; }
            let event: Value = match serde_json::from_str(&line) {
                Ok(event) => event,
                Err(error) => { parse_error = Some(format!("answer worker emitted invalid stream JSON: {error}")); break; }
            };
            match event.get("event").and_then(Value::as_str) {
                Some("delta") => {
                    if let Some(delta) = event.get("text").and_then(Value::as_str) {
                        append_job_delta(&worker_jobs, &worker_id, delta);
                    }
                }
                Some("thinking") => {
                    if let Some(chars) = event.get("chars").and_then(Value::as_u64) {
                        append_job_thinking(&worker_jobs, &worker_id, chars as usize);
                    }
                }
                Some("result") => { result = event.get("result").cloned(); }
                _ => {}
            }
        }
        let output = match child.wait_with_output() {
            Ok(output) => output,
            Err(error) => {
                set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":"failed","error":format!("could not finish answer worker: {error}")}));
                return;
            }
        };
        if let Some(error) = parse_error {
            set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":"failed","error":error,"stderr":String::from_utf8_lossy(&output.stderr).trim()}));
            return;
        }
        let result = match result {
            Some(result) => result,
            None => {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
                set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":"failed","error":if detail.is_empty() { format!("answer worker exited without a final result ({})",output.status) } else { detail }}));
                return;
            }
        };
        if result.get("status").and_then(Value::as_str) == Some("ready") {
            let answer = result.get("answer").and_then(Value::as_str).unwrap_or("");
            let answer_model = result.get("model").and_then(Value::as_str);
            let sources = result.get("sources");
            let saved = KnowledgeStore::open(&db_path).and_then(|knowledge| knowledge.append_message_with_sources(&worker_conversation,"assistant",answer,answer_model,sources));
            if let Err(error) = saved {
                set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":"failed","error":format!("answer generated but could not be saved: {error}"),"generation":result}));
                return;
            }
            set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":"ready","result":result}));
        } else {
            let status = result.get("status").and_then(Value::as_str).unwrap_or("unavailable");
            set_job(&worker_jobs,&worker_id,json!({"id":worker_id,"status":if status == "unavailable" {"unavailable"} else {"failed"},"result":result}));
        }
    });
    job_id
}

fn request_security_error(request: &Request, bind_address: &str, token: &str) -> Option<(u16, &'static str)> {
    let header_value = |name: &str| {
        request.headers().iter()
            .find(|item| item.field.as_str().as_bytes().eq_ignore_ascii_case(name.as_bytes()))
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

    if request.method() == &Method::Options { return None; }
    let path = request.url().split('?').next().unwrap_or("/");
    let is_health = (path == "/api/health" || path == "/") && request.method() == &Method::Get;
    if !is_health {
        let authorization = header_value("Authorization").unwrap_or("");
        let supplied = authorization.strip_prefix("Bearer ").unwrap_or("");
        if !constant_time_eq(supplied, token) {
            return Some((401, "missing or invalid API token"));
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

fn normalize_http_url(raw: &str) -> std::result::Result<String, &'static str> {
    let mut parsed = Url::parse(raw.trim()).map_err(|_| "URL must be an absolute HTTP or HTTPS URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none()
        || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("only HTTP and HTTPS URLs without embedded credentials are supported");
    }
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}
fn normalized_domain(raw: &str) -> Option<String> {
    let value = raw.trim().trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty() || value.contains('/') || value.contains('*') || value.contains(':') { return None; }
    let parsed = Url::parse(&format!("http://{value}/")).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    (host == value).then_some(host)
}
fn url_domain_matches(url: &str, domain: &str) -> bool {
    let host = Url::parse(url).ok().and_then(|value| value.host_str().map(str::to_ascii_lowercase));
    match (host, normalized_domain(domain)) {
        (Some(host), Some(domain)) => host == domain || host.ends_with(&format!(".{domain}")),
        _ => false,
    }
}
fn url_matches_patterns(url: &str, patterns: &[String]) -> bool {
    let parsed = match Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    let options = MatchOptions {
        case_sensitive: false,
        require_literal_leading_dot: false,
        require_literal_separator: true,
    };

    let matches = |candidate: &str, pattern: &str| {
        Pattern::new(pattern)
            .map(|compiled| compiled.matches_with(candidate, options))
            .unwrap_or(false)
    };

    patterns.iter().filter_map(|pattern| {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return None;
        }

        // URL patterns are matched by component. In particular, a path wildcard
        // must not consume a slash into a different path segment, while query
        // strings must not make an otherwise matching path fail.
        let (scheme_pattern, remainder) = match pattern.split_once("://") {
            Some(parts) => parts,
            None => return Some(matches(url, pattern)),
        };
        let (authority_pattern, path_pattern) = remainder
            .split_once('/')
            .map(|(authority, path)| (authority, Some(format!("/{path}"))))
            .unwrap_or((remainder, None));

        let host = parsed.host_str()?;
        let authority = match parsed.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };

        let scheme_matches = matches!(parsed.scheme(), "http" | "https")
            && matches(parsed.scheme(), scheme_pattern);
        let authority_matches = matches(&authority, authority_pattern);
        let path_matches = match path_pattern {
            None => true,
            Some(path_pattern) => {
                let (path_glob, query_glob) = path_pattern
                    .split_once('?')
                    .map(|(path, query)| (path, Some(query)))
                    .unwrap_or((&path_pattern, None));
                let path = parsed.path();
                // Match the complete path first. If a trailing wildcard is
                // intended to cover a route, also test each segment boundary.
                // This makes "/checkout*" cover "/checkout/order" without
                // allowing it to match unrelated paths such as "/blog/checkout".
                let path_matches = matches(path, path_glob)
                    || path.match_indices('/')
                        .skip(1)
                        .any(|(index, _)| matches(&path[..index], path_glob));
                let query_matches = query_glob.map(|query_glob| {
                    matches(parsed.query().unwrap_or(""), query_glob)
                }).unwrap_or(true);
                path_matches && query_matches
            }
        };

        Some(scheme_matches && authority_matches && path_matches)
    }).any(|matched| matched)
}
const BUILTIN_EXCLUDED_DOMAINS: &[&str] = &[
    "paypal.com", "stripe.com", "chase.com", "bankofamerica.com",
    "wellsfargo.com", "accounts.google.com", "login.microsoftonline.com",
];
const BUILTIN_EXCLUDED_PATTERNS: &[&str] = &[
    "*://*/login*", "*://*/signin*", "*://*/checkout*",
    "*://*/payment*", "*://*/account*", "*://*/settings*",
];

fn web_rule_decision(web: &crate::config::WebConfig, url: &str) -> std::result::Result<String, String> {
    let normalized = normalize_http_url(url).map_err(str::to_string)?;
    if BUILTIN_EXCLUDED_DOMAINS.iter().any(|domain| url_domain_matches(&normalized, domain))
        || web.exclude_domains.iter().any(|domain| url_domain_matches(&normalized, domain)) {
        return Err("domain is excluded".into());
    }
    if !web.allow_domains.is_empty() && !web.allow_domains.iter().any(|domain| url_domain_matches(&normalized, domain)) { return Err("domain is not in the allow list".into()); }
    let builtin_patterns: Vec<String> = BUILTIN_EXCLUDED_PATTERNS.iter().map(|value| (*value).to_string()).collect();
    if url_matches_patterns(&normalized, &builtin_patterns) || url_matches_patterns(&normalized, &web.exclude_patterns) {
        return Err("URL matches an excluded pattern".into());
    }
    Ok(normalized)
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

    if parts.as_slice() == ["api", "rules"] && method == Method::Get {
        json(request, 200, json!({"rules":cfg.settings.web,"protected_domains":BUILTIN_EXCLUDED_DOMAINS,"protected_patterns":BUILTIN_EXCLUDED_PATTERNS})); return;
    }
    if parts.as_slice() == ["api", "rules", "test"] && method == Method::Post {
        let body = match read_body(&mut request) { Ok(v) => v, Err(e) => { json(request,400,json!({"error":e.to_string()})); return; } };
        let input: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let raw = input.get("url").and_then(Value::as_str).unwrap_or("");
        match web_rule_decision(&cfg.settings.web, raw) {
            Ok(normalized) => json(request,200,json!({"allowed":true,"normalized_url":normalized})),
            Err(reason) => json(request,200,json!({"allowed":false,"reason":reason})),
        }
        return;
    }
    if parts.as_slice() == ["api", "rules", "ignore"] && method == Method::Post {
        let body = match read_body(&mut request) { Ok(v) => v, Err(e) => { json(request,400,json!({"error":e.to_string()})); return; } };
        let input: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let raw = input.get("url").and_then(Value::as_str).unwrap_or("");
        let host = match Url::parse(raw).ok().filter(|v| matches!(v.scheme(),"http"|"https")).and_then(|v| v.host_str().map(str::to_ascii_lowercase)) {
            Some(v) => v, None => { json(request,400,json!({"error":"valid HTTP or HTTPS URL required"})); return; }
        };
        let mut settings = cfg.settings.clone();
        if !settings.web.exclude_domains.iter().any(|v| normalized_domain(v).as_deref() == Some(host.as_str())) { settings.web.exclude_domains.push(host.clone()); }
        match cfg.write_settings(&settings) { Ok(()) => json(request,200,json!({"status":"ignored","domain":host})), Err(e) => json(request,500,json!({"error":e})) }
        return;
    }
    if parts.as_slice() == ["api", "rules"] && method == Method::Put {
        let body = match read_body(&mut request) { Ok(v) => v, Err(e) => { json(request,400,json!({"error":e.to_string()})); return; } };
        let input: Value = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { json(request,400,json!({"error":e.to_string()})); return; } };
        let mut settings = cfg.settings.clone();
        let web = &mut settings.web;
        if let Some(values) = input.get("allow_domains").and_then(Value::as_array) {
            let mut out = Vec::new();
            for item in values { let raw = item.as_str().unwrap_or(""); let Some(value) = normalized_domain(raw) else { json(request,400,json!({"error":format!("invalid allowed domain: {raw}")})); return; }; if !out.contains(&value) { out.push(value); } }
            web.allow_domains = out;
        }
        if let Some(values) = input.get("exclude_domains").and_then(Value::as_array) {
            let mut out = Vec::new();
            for item in values { let raw = item.as_str().unwrap_or(""); let Some(value) = normalized_domain(raw) else { json(request,400,json!({"error":format!("invalid excluded domain: {raw}")})); return; }; if !out.contains(&value) { out.push(value); } }
            web.exclude_domains = out;
        }
        if let Some(values) = input.get("exclude_patterns").and_then(Value::as_array) {
            let mut out = Vec::new();
            for item in values { let raw = item.as_str().unwrap_or("").trim(); if raw.is_empty() { continue; } if Pattern::new(raw).is_err() { json(request,400,json!({"error":format!("invalid glob pattern: {raw}")})); return; } out.push(raw.to_string()); }
            web.exclude_patterns = out;
        }
        if let Some(v) = input.get("minimum_text_length").and_then(Value::as_u64) { web.minimum_text_length = (v as usize).clamp(100,100000); }
        if let Some(v) = input.get("minimum_paragraphs").and_then(Value::as_u64) { web.minimum_paragraphs = (v as usize).clamp(1,500); }
        if let Some(v) = input.get("max_page_size_mb").and_then(Value::as_u64) { web.max_page_size_mb = (v as usize).clamp(1,100); }
        match cfg.write_settings(&settings) { Ok(()) => json(request,200,json!({"status":"updated","rules":settings.web})), Err(e) => json(request,500,json!({"error":e})) }
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
        let after = query_parameter(&url, "after")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        match job_snapshot(jobs, id, after) {
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

    if parts.len() == 4 && parts[0] == "api" && parts[1] == "conversations" && parts[3] == "regenerate" && method == Method::Post {
        let conversation_id = parts[2];
        let input: Value = read_body(&mut request).ok().and_then(|body| serde_json::from_str(&body).ok()).unwrap_or_else(|| json!({}));
        let selected_model = input.get("model").and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).unwrap_or(&cfg.settings.answer.model).to_string();
        match store.get_messages(conversation_id) {
            Ok(messages) => {
                let latest_assistant = messages.last().filter(|message| message.role == "assistant");
                let question = if let Some(answer) = latest_assistant {
                    let question = messages.iter().rev().skip(1).find(|message| message.role == "user").map(|message| message.content.clone());
                    if store.delete_message(conversation_id, &answer.id).unwrap_or(false) { question } else { None }
                } else if messages.last().map(|message| message.role == "user").unwrap_or(false) {
                    messages.iter().rev().find(|message| message.role == "user").map(|message| message.content.clone())
                } else { None };
                let Some(question) = question else { json(request,409,json!({"error":"there is no latest answer or unanswered user message to regenerate"})); return; };
                let grounding = input.get("grounding").and_then(Value::as_bool).unwrap_or(true);
                let job_id = queue_answer_job(jobs,conversation_id,question,selected_model,grounding,cfg.db_path.clone());
                json(request,202,json!({"status":"queued","job_id":job_id,"conversation_id":conversation_id}));
            },
            Err(error) => json(request,500,json!({"error":error.to_string()})),
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
        let selected_model = input.get("model").and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).unwrap_or(&cfg.settings.answer.model).to_string();
        let grounding = input.get("grounding").and_then(Value::as_bool).unwrap_or(true);
        let job_id = queue_answer_job(jobs, conversation_id, question, selected_model, grounding, cfg.db_path.clone());
        json(
            request,
            202,
            json!({"status":"queued","job_id":job_id,"conversation_id":conversation_id}),
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
        let canonical_raw = input.get("canonical_url").and_then(Value::as_str).filter(|v| !v.trim().is_empty()).unwrap_or(url);
        let manual = input.get("manual").and_then(Value::as_bool).unwrap_or(false);
        if !cfg.settings.web.enabled || (!cfg.settings.web.auto_index && !manual) { json(request,403,json!({"error":"web indexing is disabled in Hydrogen configuration"})); return; }
        if url.is_empty() || title.trim().is_empty() || content.trim().is_empty() { json(request,400,json!({"error":"url, title and readable content are required"})); return; }
        let normalized_url = match normalize_http_url(url) { Ok(v) => v, Err(e) => { json(request,400,json!({"error":e})); return; } };
        let canonical = match normalize_http_url(canonical_raw) { Ok(v) => v, Err(e) => { json(request,400,json!({"error":format!("invalid canonical URL: {e}")})); return; } };
        if let Err(reason) = web_rule_decision(&cfg.settings.web, &normalized_url).and_then(|_| web_rule_decision(&cfg.settings.web, &canonical)) { json(request,403,json!({"error":format!("page blocked by web capture rules: {reason}")})); return; }
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
            &normalized_url,
            &canonical,
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
                    let _ = store.record_visit(&canonical);
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
    let token = api_token(cfg).map_err(|error| HydrogenError::Config(format!("could not initialise API token: {error}")))?;
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
        if let Some((status, message)) = request_security_error(&request, address, &token) {
            json(request, status, json!({"error":message}));
            continue;
        }
        if request.method() == &Method::Options {
            respond(request, 204, "application/json; charset=utf-8", String::new());
            continue;
        }
        if request.method() == &Method::Get && (url == "/" || url.starts_with("/?")) {
            respond(
                request,
                200,
                "text/html; charset=utf-8",
                FRONTEND.replace("__HYDROGEN_API_TOKEN__", &token),
            );
        } else if url.starts_with("/api/") {
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
    let _token = api_token(cfg).map_err(|error| HydrogenError::Config(format!("could not initialise API token: {error}")))?;
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

    #[test]
    fn job_polling_returns_only_new_deltas() {
        let jobs: super::JobStore = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        super::set_job(&jobs, "job_test", serde_json::json!({
            "id":"job_test","status":"generating","answer":"","chunks":[],"seq":0,
            "created_at":super::unix_seconds()
        }));
        super::append_job_delta(&jobs, "job_test", "hello ");
        super::append_job_delta(&jobs, "job_test", "world");
        let first = super::job_snapshot(&jobs, "job_test", 0).unwrap();
        assert_eq!(first["delta"], "hello world");
        assert_eq!(first["seq"], 2);
        assert!(first.get("answer").is_none());
        let next = super::job_snapshot(&jobs, "job_test", 2).unwrap();
        assert_eq!(next["delta"], "");
        assert_eq!(next["seq"], 2);
    }

    #[test]
    fn job_polling_resets_when_the_client_cursor_is_too_old() {
        let jobs: super::JobStore = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        super::set_job(&jobs, "job_test", serde_json::json!({
            "id":"job_test","status":"generating","answer":"","chunks":[],"seq":0,
            "created_at":super::unix_seconds()
        }));
        for index in 0..=super::JOB_MAX_CHUNKS {
            super::append_job_delta(&jobs, "job_test", &format!("{index},"));
        }
        let snapshot = super::job_snapshot(&jobs, "job_test", 0).unwrap();
        assert_eq!(snapshot["reset"], true);
        assert_eq!(snapshot["answer"].as_str().unwrap().split(',').count(), super::JOB_MAX_CHUNKS + 2);
        assert_eq!(snapshot["seq"], (super::JOB_MAX_CHUNKS + 1) as u64);
    }

    #[test]
    fn failed_jobs_keep_partial_answers_and_expired_jobs_are_pruned() {
        let jobs: super::JobStore = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        super::set_job(&jobs, "job_test", serde_json::json!({
            "id":"job_test","status":"streaming","answer":"","chunks":[],"seq":0,
            "created_at":super::unix_seconds()
        }));
        super::append_job_delta(&jobs, "job_test", "partial output");
        super::set_job(&jobs, "job_test", serde_json::json!({
            "id":"job_test","status":"failed","error":"worker stopped"
        }));
        let snapshot = super::job_snapshot(&jobs, "job_test", 1).unwrap();
        assert_eq!(snapshot["answer"], "partial output");
        assert_eq!(snapshot["status"], "failed");

        let mut stored = std::collections::HashMap::new();
        stored.insert("expired".to_string(), serde_json::json!({
            "status":"failed","created_at":super::unix_seconds().saturating_sub(super::JOB_TTL_SECONDS + 1)
        }));
        stored.insert("active".to_string(), serde_json::json!({
            "status":"streaming","created_at":super::unix_seconds().saturating_sub(super::JOB_TTL_SECONDS + 1)
        }));
        super::prune_jobs(&mut stored);
        assert!(!stored.contains_key("expired"));
        assert!(stored.contains_key("active"));
    }

    #[test]
    fn api_token_comparison_requires_exact_secret_match() {
        assert!(super::constant_time_eq("0123456789abcdef", "0123456789abcdef"));
        assert!(!super::constant_time_eq("0123456789abcdef", "0123456789abcdeg"));
        assert!(!super::constant_time_eq("", "0123456789abcdef"));
    }

    #[test]
    fn url_normalization_removes_fragments_and_default_ports() {
        assert_eq!(super::normalize_http_url("HTTPS://Example.COM:443/article#section").unwrap(), "https://example.com/article");
        assert!(super::normalize_http_url("file:///etc/passwd").is_err());
        assert!(super::normalize_http_url("https://user:pass@example.com/").is_err());
    }

    #[test]
    fn domain_rules_match_only_exact_hosts_and_subdomains() {
        assert!(super::url_domain_matches("https://blog.example.com/post", "example.com"));
        assert!(super::url_domain_matches("https://example.com/post", "example.com"));
        assert!(!super::url_domain_matches("https://notexample.com/post", "example.com"));
    }

    #[test]
    fn empty_patterns_do_not_exclude_every_url_and_globs_match_paths() {
        assert!(!super::url_matches_patterns("https://example.com/article", &[]));
        assert!(!super::url_matches_patterns("https://example.com/article", &["".into()]));
        assert!(super::url_matches_patterns("https://example.com/login?next=/home", &["*://*/login*".into()]));
        assert!(!super::url_matches_patterns("https://example.com/blog/login-tips", &["*://*/login*".into()]));
    }

    #[test]
    fn web_rules_respect_allow_and_exclude_lists() {
        let mut web = crate::config::WebConfig::default();
        web.exclude_domains.clear();
        web.exclude_patterns.clear();
        web.allow_domains = vec!["example.com".into()];
        assert!(super::web_rule_decision(&web, "https://blog.example.com/post").is_ok());
        assert!(super::web_rule_decision(&web, "https://other.test/post").is_err());
        web.exclude_domains.push("private.example.com".into());
        assert!(super::web_rule_decision(&web, "https://private.example.com/post").is_err());
        assert!(super::web_rule_decision(&web, "https://www.paypal.com/login").is_err());
        assert!(super::web_rule_decision(&web, "https://example.com/checkout/order").is_err());
    }
}
