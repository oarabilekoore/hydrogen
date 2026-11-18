use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "avif"];

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub mtime: i64,
    pub size: u64,
    pub is_dir: bool,
    pub dev: u64,
    pub ino: u64,
}

#[derive(Debug, Clone)]
pub struct CachedImage {
    pub path: String,
    pub mtime: i64,
    pub size: u64,
    pub ocr_text: String,
    pub caption_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    pub source: String,
    pub score: f64,
    pub vec: f64,
    pub fts: f64,
    pub coverage: f64,
    pub phrase: bool,
    pub snippet: String,
    #[serde(rename = "isDir")]
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub indexed_files: usize,
    pub indexed_images: usize,
    pub captioned_images: usize,
    pub ocr_images: usize,
    pub image_sync_running: bool,
    pub posting_rows: usize,
    pub idf_terms: usize,
    pub db_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSummary {
    pub total: usize,
    pub updated: usize,
    pub moved: usize,
    pub removed: usize,
    pub pending_images: usize,
    pub image_worker_started: bool,
}

pub fn is_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let low = ext.to_ascii_lowercase();
            IMAGE_EXTS.contains(&low.as_str())
        })
        .unwrap_or(false)
}

pub fn compose_image_body(caption_text: &str, ocr_text: &str) -> String {
    let mut parts = Vec::with_capacity(2);
    let cap = caption_text.trim();
    let ocr = ocr_text.trim();
    if !cap.is_empty() {
        parts.push(format!("[Caption] {cap}"));
    }
    if !ocr.is_empty() {
        parts.push(format!("[OCR] {ocr}"));
    }
    parts.join("\n")
}
