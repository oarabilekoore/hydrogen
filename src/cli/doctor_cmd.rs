use crate::config::AppConfig;
use crate::error::Result;
use rusqlite::Connection;\nuse crate::adapters::sqlite::repo::SqliteIndexRepository;
use serde_json::{json, Value};
use std::env;
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

fn command_available(name: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

fn directory_bytes(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

fn memory_available_bytes() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    contents.lines().find_map(|line| {
        let value = line.strip_prefix("MemAvailable:")?.split_whitespace().next()?;
        value.parse::<u64>().ok().map(|kib| kib * 1024)
    })
}

pub fn run(cfg: &AppConfig) -> Result<()> {
    let mut checks = Vec::new();
    let mut check = |name: &str, ok: bool, detail: String| {
        checks.push(json!({"name": name, "ok": ok, "detail": detail}));
    };

    for binary in ["pdfinfo", "pdftotext", "pdftoppm"] {
        check(
            &format!("dependency:{binary}"),
            command_available(binary),
            if command_available(binary) { "found in PATH".into() } else { "missing from PATH".into() },
        );
    }

    let det = cfg.models_dir.join("ppocr/det/inference.onnx");
    let rec = cfg.models_dir.join("ppocr/rec/inference.onnx");
    let texo = cfg.models_dir.join("texo-distill-f16.gguf");
    check("model:ppocr-detector", det.is_file(), det.display().to_string());
    check("model:ppocr-recognizer", rec.is_file(), rec.display().to_string());
    check("model:texo", texo.is_file(), texo.display().to_string());

    let embedding_dir = cfg.models_dir.join("embeddings/fastembed");
    let embedding_bytes = directory_bytes(&embedding_dir);
    check(
        "models:embedding-cache",
        embedding_bytes > 0,
        format!("{} bytes in {}", embedding_bytes, embedding_dir.display()),
    );

    let mut database = json!({"exists": cfg.db_path.is_file()});
    if cfg.db_path.is_file() {
        let _repo = SqliteIndexRepository::open(&cfg.db_path)?;\n        let conn = Connection::open(&cfg.db_path)?;
        let integrity: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        let queue: Value = {
            let mut stmt = conn.prepare(
                "SELECT status, COUNT(*) FROM model_jobs GROUP BY status ORDER BY status",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            let mut counts = serde_json::Map::new();
            for row in rows {
                let (status, count) = row?;
                counts.insert(status, json!(count));
            }
            Value::Object(counts)
        };
        let vectors: Value = {
            let mut stmt = conn.prepare(
                "SELECT model_id || ':' || modality, COUNT(*) FROM embedding_vectors GROUP BY model_id, modality",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            let mut counts = serde_json::Map::new();
            for row in rows {
                let (model, count) = row?;
                counts.insert(model, json!(count));
            }
            Value::Object(counts)
        };
        database = json!({
            "exists": true,
            "integrity": integrity,
            "queue": queue,
            "embeddings": vectors
        });
        check(
            "database:integrity",
            integrity == "ok",
            integrity,
        );
    }

    let memory = memory_available_bytes();
    let report = json!({
        "status": if checks.iter().all(|c| c["ok"].as_bool().unwrap_or(false)) { "ok" } else { "attention" },
        "state_dir": cfg.state_dir,
        "models_dir": cfg.models_dir,
        "database": database,
        "memory_available_bytes": memory,
        "inference_threads": cfg.inference_threads,
        "image_batch_size": cfg.image_batch_size,
        "pdf_page_limit": cfg.pdf_page_limit,
        "embeddings_enabled": cfg.embeddings_enabled,
        "checks": checks,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
