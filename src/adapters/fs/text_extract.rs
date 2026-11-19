use crate::adapters::fs::image_header::get_image_dimensions;
use crate::domain::document::is_image_extension;
use crate::ports::extractor::TextExtractor;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use zip::ZipArchive;

const MAX_EXTRACT_BYTES: u64 = 15 * 1024 * 1024;
const MIN_IMAGE_BYTES: u64 = 10 * 1024;
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
const MIN_IMAGE_DIM: u32 = 128;
const MAX_TEXT_CHARS: usize = 120_000;

const TEXT_EXTS: &[&str] = &[
    "txt", "md", "markdown", "rst", "org", "tex", "typ", "py", "c", "cpp", "cc", "h", "hpp", "go",
    "rs", "ts", "tsx", "js", "jsx", "v", "odin", "java", "kt", "sh", "fish", "zsh", "bash", "qml",
    "json", "toml", "yaml", "yml", "csv", "tsv", "html", "htm", "xml", "css", "scss", "sql", "ini",
    "conf", "cfg", "log",
];

const XML_ZIP_EXTS: &[&str] = &["docx", "odt", "epub", "pptx", "xlsx"];

const IMAGE_SKIP_DIRS: &[&str] = &[
    "assets",
    "asset",
    "cursors",
    "cursor",
    "icons",
    "icon",
    "themes",
    "theme",
    "sprites",
    "thumbnails",
    ".thumbnails",
];

pub struct CompositeTextExtractor;

impl TextExtractor for CompositeTextExtractor {
    fn extract_text(&self, path: &Path, size: u64) -> String {
        if size == 0 || size > MAX_EXTRACT_BYTES {
            return String::new();
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return String::new();
        };
        let low = ext.to_ascii_lowercase();

        if low == "pdf" {
            return extract_pdf_text(path);
        }
        if XML_ZIP_EXTS.contains(&low.as_str()) {
            return extract_xml_zip_text(path);
        }
        if TEXT_EXTS.contains(&low.as_str()) {
            if let Ok(bytes) = std::fs::read(path) {
                return String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(MAX_TEXT_CHARS)
                    .collect();
            }
        }
        String::new()
    }

    fn is_indexable_image(&self, path: &Path, size: u64) -> bool {
        if !is_image_extension(path) {
            return false;
        }
        if !(MIN_IMAGE_BYTES..=MAX_IMAGE_BYTES).contains(&size) {
            return false;
        }
        if let Some(parent) = path.parent() {
            for comp in parent.components() {
                if let Some(s) = comp.as_os_str().to_str() {
                    if IMAGE_SKIP_DIRS.contains(&s.to_ascii_lowercase().as_str()) {
                        return false;
                    }
                }
            }
        }
        if let Some((w, h)) = get_image_dimensions(path) {
            if w < MIN_IMAGE_DIM || h < MIN_IMAGE_DIM {
                return false;
            }
        }
        true
    }
}

fn extract_pdf_text(path: &Path) -> String {
    let Ok(out) = Command::new("pdftotext")
        .args(["-q", "-enc", "UTF-8"])
        .arg(path)
        .arg("-")
        .stderr(Stdio::null())
        .output()
    else {
        return String::new();
    };
    if out.status.success() {
        String::from_utf8_lossy(&out.stdout)
            .chars()
            .take(MAX_TEXT_CHARS)
            .collect()
    } else {
        String::new()
    }
}

fn extract_xml_zip_text(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let Ok(mut archive) = ZipArchive::new(file) else {
        return String::new();
    };

    let mut chunks: Vec<String> = Vec::new();
    let mut total_chars = 0usize;

    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            continue;
        };
        if !entry.name().ends_with(".xml") {
            continue;
        }
        let mut xml_bytes = Vec::new();
        if entry.read_to_end(&mut xml_bytes).is_err() {
            continue;
        }
        let xml_str = String::from_utf8_lossy(&xml_bytes);
        let mut reader = Reader::from_str(&xml_str);
        loop {
            match reader.read_event() {
                Ok(Event::Text(e)) => {
                    if let Ok(txt) = e.unescape() {
                        let t = txt.trim();
                        if !t.is_empty() {
                            total_chars += t.chars().count();
                            chunks.push(t.to_string());
                            if total_chars >= MAX_TEXT_CHARS {
                                return chunks.join(" ").chars().take(MAX_TEXT_CHARS).collect();
                            }
                        }
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
    }

    chunks.join(" ").chars().take(MAX_TEXT_CHARS).collect()
}

/// Extract each PDF page independently and render a small local preview.
/// Requires Poppler's pdftotext, pdfinfo, and pdftoppm utilities.
pub fn extract_pdf_pages(path: &Path, preview_root: &Path) -> Vec<(i64, String, String)> {
    let info = Command::new("pdfinfo")
        .arg(path)
        .stderr(Stdio::null())
        .output();
    let Ok(info) = info else {
        return Vec::new();
    };
    if !info.status.success() {
        return Vec::new();
    }
    let info_text = String::from_utf8_lossy(&info.stdout);
    let Some(page_count) = info_text.lines().find_map(|line| {
        line.strip_prefix("Pages:")
            .and_then(|v| v.trim().parse::<u32>().ok())
    }) else {
        return Vec::new();
    };
    let key = blake3::hash(path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    let dir = preview_root.join("pdf-pages").join(key);
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let mut pages = Vec::new();
    for page in 1..=page_count.min(500) {
        let text_out = Command::new("pdftotext")
            .args(["-q", "-enc", "UTF-8", "-layout", "-f"])
            .arg(page.to_string())
            .arg("-l")
            .arg(page.to_string())
            .arg(path)
            .arg("-")
            .stderr(Stdio::null())
            .output();
        let Ok(text_out) = text_out else {
            continue;
        };
        let mut text = if text_out.status.success() {
            String::from_utf8_lossy(&text_out.stdout)
                .chars()
                .take(40_000)
                .collect::<String>()
        } else {
            String::new()
        };
        let preview = dir.join(format!("page-{page}"));
        let rendered = Command::new("pdftoppm")
            .args(["-f"])
            .arg(page.to_string())
            .args(["-l"])
            .arg(page.to_string())
            .args(["-singlefile", "-scale-to", "1200", "-png"])
            .arg(path)
            .arg(&preview)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let preview_path = if rendered.map(|s| s.success()).unwrap_or(false) {
            preview.with_extension("png").to_string_lossy().to_string()
        } else {
            String::new()
        };
        if text.trim().is_empty() && !preview_path.is_empty() {
            // Image-only page: OCR is intentionally queued for the Rust OCR worker.
            text = String::new();
        }
        pages.push((i64::from(page), text, preview_path));
    }
    pages
}
