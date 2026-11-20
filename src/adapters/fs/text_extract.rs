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
const MAX_ARCHIVE_XML_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ARCHIVE_TOTAL_XML_BYTES: u64 = 24 * 1024 * 1024;

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

/// True for files whose indexed body is the raw file text, so a body line number
/// is also the line number in the file (not PDFs or zipped XML formats).
pub fn is_plain_text_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| TEXT_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

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
    let mut total_xml_bytes = 0u64;

    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            continue;
        };
        if !entry.name().ends_with(".xml") {
            continue;
        }
        // ZIP metadata is untrusted. Bound decompressed input per member and overall.
        let member_limit = entry.size().min(MAX_ARCHIVE_XML_BYTES);
        if entry.size() > MAX_ARCHIVE_XML_BYTES
            || total_xml_bytes.saturating_add(member_limit) > MAX_ARCHIVE_TOTAL_XML_BYTES
        {
            continue;
        }
        let mut xml_bytes = Vec::with_capacity(member_limit as usize);
        if entry
            .take(member_limit + 1)
            .read_to_end(&mut xml_bytes)
            .is_err()
            || xml_bytes.len() as u64 > member_limit
        {
            continue;
        }
        total_xml_bytes = total_xml_bytes.saturating_add(xml_bytes.len() as u64);
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

/// Extract PDF text in one Poppler pass. Page previews are rendered only
/// for image-only pages that need OCR or for pages returned by search.
pub fn extract_pdf_pages(
    path: &Path,
    _preview_root: &Path,
    max_pages: usize,
) -> Vec<(i64, String, String)> {
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
    if page_count == 0 {
        return Vec::new();
    }
    let text_out = Command::new("pdftotext")
        .args(["-q", "-enc", "UTF-8", "-layout"])
        .arg(path)
        .arg("-")
        .stderr(Stdio::null())
        .output();
    let Ok(text_out) = text_out else {
        return Vec::new();
    };
    if !text_out.status.success() {
        return Vec::new();
    }
    let extracted = String::from_utf8_lossy(&text_out.stdout);
    let page_texts: Vec<&str> = extracted.split('\u{000c}').collect();
    let mut pages = Vec::with_capacity(page_count.min(500) as usize);
    for page in 1..=page_count.min(500) {
        let text = page_texts
            .get((page - 1) as usize)
            .copied()
            .unwrap_or("")
            .chars()
            .take(40_000)
            .collect::<String>();
        pages.push((i64::from(page), text, String::new()));
    }
    pages
}

/// Render and cache one PDF page. The content digest is part of the cache key,
/// so replacing a PDF at the same path cannot return a stale preview.
pub fn render_pdf_page(path: &Path, page_no: i64, preview_root: &Path) -> Option<String> {
    if page_no < 1 || !path.is_file() {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize().to_hex().to_string();
    let key = blake3::hash(path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    let dir = preview_root
        .join("pdf-pages")
        .join(format!("{key}-{}", &digest[..12]));
    std::fs::create_dir_all(&dir).ok()?;
    let output = dir.join(format!("page-{page_no}.png"));
    if output.is_file() {
        return Some(output.to_string_lossy().to_string());
    }
    let prefix = dir.join(format!("page-{page_no}"));
    let status = Command::new("pdftoppm")
        .args(["-f"])
        .arg(page_no.to_string())
        .args(["-l"])
        .arg(page_no.to_string())
        .args(["-singlefile", "-scale-to", "900", "-png"])
        .arg(path)
        .arg(&prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if status.success() && output.is_file() {
        Some(output.to_string_lossy().to_string())
    } else {
        None
    }
}
