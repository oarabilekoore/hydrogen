use crate::adapters::ml::embeddings::{CLIP_MODEL_ID, TEXT_MODEL_ID};
use crate::config::AppConfig;
use crate::domain::document::{
    compose_image_body, CachedImage, FileEntry, IndexStats, SyncSummary,
};
use crate::domain::search::tfidf::extract_doc_terms;
use crate::error::Result;
use crate::ports::extractor::{
    CaptionProvider, EmbeddingProvider, FileScanner, OcrProvider, TextExtractor,
};
use crate::ports::repository::IndexRepository;
use serde_json::json;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct SyncService<'a> {
    cfg: &'a AppConfig,
    repo: &'a dyn IndexRepository,
    scanner: &'a dyn FileScanner,
    extractor: &'a dyn TextExtractor,
    ocr: &'a dyn OcrProvider,
    captioner: &'a dyn CaptionProvider,
    embeddings: &'a dyn EmbeddingProvider,
}

impl<'a> SyncService<'a> {
    pub fn new(
        cfg: &'a AppConfig,
        repo: &'a dyn IndexRepository,
        scanner: &'a dyn FileScanner,
        extractor: &'a dyn TextExtractor,
        ocr: &'a dyn OcrProvider,
        captioner: &'a dyn CaptionProvider,
        embeddings: &'a dyn EmbeddingProvider,
    ) -> Self {
        Self {
            cfg,
            repo,
            scanner,
            extractor,
            ocr,
            captioner,
            embeddings,
        }
    }

    pub fn sync(&self, spawn_images: bool) -> Result<SyncSummary> {
        let sync_started = Instant::now();
        fs::create_dir_all(&self.cfg.state_dir)?;
        let entries = self.scanner.scan(6, 5000);

        let mut existing = self.repo.load_file_records()?;
        let mut content_hashes = self.repo.load_content_hashes()?;
        let mut img_cached = self.repo.load_image_cache()?;
        let current_paths: HashSet<String> = entries
            .iter()
            .map(|e| e.path.to_string_lossy().to_string())
            .collect();

        // Phase 1: Recover offline renames/moves via (dev, ino) before deleting missing paths
        let mut moved = 0usize;
        for entry in &entries {
            let p_str = entry.path.to_string_lossy().to_string();
            if existing.contains_key(&p_str) || entry.ino == 0 {
                continue;
            }
            match self.repo.find_by_inode(entry.dev, entry.ino)? {
                Some(old_path)
                    if old_path != p_str
                        && !current_paths.contains(&old_path)
                        && self.repo.rename_path(&old_path, &p_str, entry.is_dir)? > 0 =>
                {
                    existing.remove(&old_path);
                    if let Some(mut ic) = img_cached.remove(&old_path) {
                        ic.path = p_str.clone();
                        img_cached.insert(p_str.clone(), ic);
                    }
                    moved += 1;
                }
                _ => {}
            }
        }

        // Phase 2: Purge genuinely deleted paths
        let removed_paths: Vec<String> = existing
            .keys()
            .filter(|p| !current_paths.contains(*p))
            .cloned()
            .collect();
        let removed = removed_paths.len();
        if !removed_paths.is_empty() {
            self.repo.remove_paths(&removed_paths)?;
        }

        // Phase 3: Index new or modified entries
        let mut updated = 0usize;
        let mut pending_images = 0usize;

        for entry in &entries {
            let p_str = entry.path.to_string_lossy().to_string();
            let prev = existing.get(&p_str).copied();
            // Metadata is a cheap first signal, but the persisted digest is the
            // source of truth when timestamps are preserved by copies/sync tools.
            let current_hash = if entry.is_dir {
                None
            } else {
                hash_file_blake3(&entry.path).ok()
            };
            let hash_changed = if let Some(hash) = current_hash.as_deref() {
                content_hashes.get(&p_str).map(String::as_str) != Some(hash)
            } else {
                false
            };
            let is_img =
                !entry.is_dir && self.extractor.is_indexable_image(&entry.path, entry.size);
            let is_pdf = entry
                .path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("pdf"))
                .unwrap_or(false);
            let file_changed = prev != Some((entry.mtime, entry.size)) || hash_changed;
            let embedding_missing = if self.cfg.embeddings_enabled && !entry.is_dir {
                if is_img {
                    let clip_missing = !self.repo.has_embedding(&p_str, CLIP_MODEL_ID, "clip")?;
                    let text_available = img_cached
                        .get(&p_str)
                        .map(|cached| {
                            !compose_image_body(&cached.caption_text, &cached.ocr_text)
                                .trim()
                                .is_empty()
                        })
                        .unwrap_or(false);
                    let text_missing = text_available
                        && !self.repo.has_embedding(&p_str, TEXT_MODEL_ID, "text")?;
                    clip_missing || text_missing
                } else if is_pdf {
                    // Existing PDF text is backfilled by hydrogen index embed.
                    // Do not mistake a missing vector for a reason to rerun Poppler/OCR.
                    false
                } else {
                    !self.repo.has_embedding(&p_str, TEXT_MODEL_ID, "text")?
                }
            } else {
                false
            };
            let content_changed = file_changed || embedding_missing;

            if !entry.is_dir && content_changed {
                if file_changed && is_pdf {
                    self.repo.enqueue_model_job(&p_str, "pdf_pages")?;
                } else if file_changed && is_img {
                    self.repo.enqueue_model_job(&p_str, "image_ocr")?;
                }
                // For PDFs, only commit the new digest after a successful page pass.
                // Otherwise a transient Poppler failure would suppress the retry.
                if !is_pdf {
                    if let Some(hash) = current_hash.as_deref() {
                        self.repo
                            .upsert_content_hash(&p_str, hash, entry.size, entry.mtime)?;
                        content_hashes.insert(p_str.clone(), hash.to_string());
                    }
                }
            }
            let title = entry
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&p_str);
            if is_img {
                if let Some(ic) = img_cached.get(&p_str) {
                    if ic.mtime == entry.mtime && ic.size == entry.size {
                        if !content_changed {
                            continue;
                        }
                        let body = compose_image_body(&ic.caption_text, &ic.ocr_text);
                        let has_body = !body.is_empty();
                        let terms = extract_doc_terms(title, &ic.ocr_text, &ic.caption_text);
                        self.repo
                            .upsert_document(entry, title, &body, has_body, &terms)?;
                        if self.cfg.embeddings_enabled {
                            if !body.trim().is_empty() {
                                match self.embeddings.embed_document(&body) {
                                    Ok(vector) => {
                                        if let Some(hash) = current_hash.as_deref() {
                                            self.repo.upsert_embedding(
                                                &p_str,
                                                TEXT_MODEL_ID,
                                                "text",
                                                &vector,
                                                hash,
                                            )?;
                                        }
                                    }
                                    Err(error) => eprintln!(
                                        "hydrogen: image text embedding failed for {p_str}: {error}"
                                    ),
                                }
                            }
                            match self.embeddings.embed_image(&entry.path) {
                                Ok(vector) => {
                                    if let Some(hash) = current_hash.as_deref() {
                                        self.repo.upsert_embedding(
                                            &p_str,
                                            CLIP_MODEL_ID,
                                            "clip",
                                            &vector,
                                            hash,
                                        )?;
                                    }
                                }
                                Err(error) => eprintln!(
                                    "hydrogen: image embedding failed for {p_str}: {error}"
                                ),
                            }
                        }
                        pending_images += 1;
                        updated += 1;
                        continue;
                    }
                }

                pending_images += 1;
                if !content_changed {
                    continue;
                }
                let terms = extract_doc_terms(title, "", "");
                self.repo.upsert_document(entry, title, "", false, &terms)?;
                updated += 1;
            } else {
                if !content_changed {
                    continue;
                }
                let body = if entry.is_dir {
                    String::new()
                } else if entry
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
                {
                    if !self.repo.claim_model_job(&p_str, "pdf_pages")? {
                        continue;
                    }
                    let pdf_started = Instant::now();
                    let pages = crate::adapters::fs::text_extract::extract_pdf_pages(
                        &entry.path,
                        &self.cfg.state_dir,
                        self.cfg.pdf_page_limit,
                    );
                    let hash = hash_file_blake3(&entry.path).unwrap_or_default();
                    let mut indexed = Vec::with_capacity(pages.len());
                    let mut extraction_failed = false;
                    for (page_no, mut page_text, preview_path) in pages {
                        let preview_path = if page_text.trim().is_empty() {
                            crate::adapters::fs::text_extract::render_pdf_page(
                                &entry.path,
                                page_no,
                                &self.cfg.state_dir,
                            )
                            .unwrap_or(preview_path)
                        } else {
                            preview_path
                        };
                        if page_text.trim().is_empty() && !preview_path.is_empty() {
                            match self.ocr.ocr_boxes(Path::new(&preview_path)) {
                                Ok(boxes) => {
                                    page_text = boxes
                                        .into_iter()
                                        .map(|b| b.text)
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                }
                                Err(error) => {
                                    eprintln!(
                                        "hydrogen: PDF page OCR failed for {p_str} page {page_no}: {error}"
                                    );
                                    extraction_failed = true;
                                }
                            }
                        } else if page_text.trim().is_empty() {
                            extraction_failed = true;
                        }
                        indexed.push((page_no, page_text, preview_path));
                    }
                    // Preserve the last known-good page and document index if Poppler
                    // or OCR fails. The next sync retries instead of recording a false success.
                    if extraction_failed || indexed.is_empty() {
                        self.repo.fail_model_job(&p_str, "pdf_pages")?;
                        self.repo.record_worker_metric(
                            "pdf_index_failed",
                            pdf_started.elapsed().as_millis() as u64,
                            current_rss_bytes(),
                            indexed.len(),
                        )?;
                        continue;
                    }
                    self.repo.replace_pdf_pages(&p_str, &indexed, &hash)?;
                    self.repo
                        .upsert_content_hash(&p_str, &hash, entry.size, entry.mtime)?;
                    self.repo.complete_model_job(&p_str, "pdf_pages")?;
                    self.repo.record_worker_metric(
                        "pdf_index",
                        pdf_started.elapsed().as_millis() as u64,
                        current_rss_bytes(),
                        indexed.len(),
                    )?;
                    content_hashes.insert(p_str.clone(), hash.clone());
                    indexed
                        .iter()
                        .map(|(n, text, _)| format!("[Page {n}] {text}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    self.extractor.extract_text(&entry.path, entry.size)
                };
                let has_body = !body.trim().is_empty();
                let terms = extract_doc_terms(title, &body, "");
                self.repo
                    .upsert_document(entry, title, &body, has_body, &terms)?;
                if self.cfg.embeddings_enabled && has_body {
                    match self.embeddings.embed_document(&body) {
                        Ok(vector) => {
                            if let Some(hash) = current_hash.as_deref() {
                                self.repo.upsert_embedding(
                                    &p_str,
                                    TEXT_MODEL_ID,
                                    "text",
                                    &vector,
                                    hash,
                                )?;
                            }
                        }
                        Err(error) => {
                            eprintln!("hydrogen: document embedding failed for {p_str}: {error}")
                        }
                    }
                }
                updated += 1;
            }
        }

        self.repo
            .rebuild_sparse_index_if_needed(updated > 0 || moved > 0 || removed > 0)?;
        if updated > 0 || moved > 0 || removed > 0 {
            self.repo.record_worker_metric(
                "document_sync",
                sync_started.elapsed().as_millis() as u64,
                current_rss_bytes(),
                updated,
            )?;
        }

        write_compat_files(&self.cfg.cache_txt, &self.cfg.rev_file, &entries)?;

        let bg_spawned = if spawn_images && pending_images > 0 {
            self.spawn_background_image_sync()
        } else {
            false
        };

        Ok(SyncSummary {
            total: entries.len(),
            updated,
            moved,
            removed,
            pending_images,
            image_worker_started: bg_spawned,
        })
    }

    pub fn sync_images(&self, retag: bool, limit: usize) -> Result<String> {
        if self.is_image_sync_running() {
            return Ok(json!({ "status": "already_running" }).to_string());
        }

        fs::create_dir_all(&self.cfg.state_dir)?;
        fs::write(&self.cfg.img_lock_path, std::process::id().to_string())?;

        let run_result = (|| -> Result<usize> {
            unsafe {
                libc::nice(19);
            }

            let entries = self.scanner.scan(6, 5000);
            let img_cached = self.repo.load_image_cache()?;
            let mut queue: Vec<(FileEntry, bool, bool)> = Vec::new();

            for entry in entries {
                if entry.is_dir || !self.extractor.is_indexable_image(&entry.path, entry.size) {
                    continue;
                }
                let p_str = entry.path.to_string_lossy().to_string();
                let needs_clip_embedding = if self.cfg.embeddings_enabled {
                    let clip_missing = !self.repo.has_embedding(&p_str, CLIP_MODEL_ID, "clip")?;
                    let text_available = img_cached
                        .get(&p_str)
                        .map(|cached| {
                            !compose_image_body(&cached.caption_text, &cached.ocr_text)
                                .trim()
                                .is_empty()
                        })
                        .unwrap_or(false);
                    let text_missing = text_available
                        && !self.repo.has_embedding(&p_str, TEXT_MODEL_ID, "text")?;
                    clip_missing || text_missing
                } else {
                    false
                };
                let ocr_pending = self.repo.model_job_pending(&p_str, "image_ocr")?;
                match img_cached.get(&p_str) {
                    None => queue.push((entry, true, true)),
                    Some(ic) if ic.mtime != entry.mtime || ic.size != entry.size => {
                        queue.push((entry, true, true));
                    }
                    Some(_ic) if ocr_pending => {
                        queue.push((entry, true, true));
                    }
                    Some(ic) if needs_clip_embedding => {
                        queue.push((entry, false, true));
                    }
                    Some(ic) if retag && ic.caption_text.trim().is_empty() => {
                        queue.push((entry, false, true));
                    }
                    _ => {}
                }
            }

            if limit > 0 && queue.len() > limit {
                queue.truncate(limit);
            }

            let mut processed = 0usize;
            for batch in queue.chunks(self.cfg.image_batch_size) {
                let batch_started = Instant::now();
                let mut claimed = HashSet::new();
                for (entry, need_ocr, _) in batch {
                    if *need_ocr {
                        let path = entry.path.to_string_lossy().to_string();
                        self.repo.enqueue_model_job(&path, "image_ocr")?;
                        if self.repo.claim_model_job(&path, "image_ocr")? {
                            claimed.insert(path);
                        }
                    }
                }
                let ocr_targets: Vec<String> = batch
                    .iter()
                    .filter(|(entry, need_ocr, _)| {
                        *need_ocr && claimed.contains(&entry.path.to_string_lossy().to_string())
                    })
                    .map(|(e, _, _)| e.path.to_string_lossy().to_string())
                    .collect();
                let cap_targets: Vec<String> = batch
                    .iter()
                    .filter(|(_, _, need_cap)| *need_cap)
                    .map(|(e, _, _)| e.path.to_string_lossy().to_string())
                    .collect();

                let ocr_map = self.ocr.batch_ocr_text(&ocr_targets);
                let cap_map = self.captioner.batch_captions(&cap_targets);

                for (entry, need_ocr, need_cap) in batch {
                    let p_str = entry.path.to_string_lossy().to_string();
                    let prev_ic = img_cached.get(&p_str);
                    // Jobs already claimed by another process are left untouched.
                    if *need_ocr && !claimed.contains(&p_str) {
                        continue;
                    }
                    // A missing map entry means inference failed. Preserve any
                    // previous OCR text, but still retain an independent CLIP vector
                    // and successful caption so one failed model cannot block retrieval.
                    let ocr_failed = *need_ocr && !ocr_map.contains_key(&p_str);
                    if ocr_failed {
                        eprintln!("hydrogen: image OCR failed; leaving {} pending", p_str);
                    }
                    let ocr_txt = if *need_ocr && !ocr_failed {
                        ocr_map.get(&p_str).cloned().unwrap_or_default()
                    } else {
                        prev_ic.map(|c| c.ocr_text.clone()).unwrap_or_default()
                    };
                    let cap_txt = if *need_cap {
                        cap_map.get(&p_str).cloned().unwrap_or_default()
                    } else {
                        prev_ic.map(|c| c.caption_text.clone()).unwrap_or_default()
                    };

                    let title = entry
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&p_str);
                    let body = compose_image_body(&cap_txt, &ocr_txt);
                    let has_body = !body.is_empty();
                    let terms = extract_doc_terms(title, &ocr_txt, &cap_txt);

                    self.repo.upsert_image_cache(&CachedImage {
                        path: p_str.clone(),
                        mtime: entry.mtime,
                        size: entry.size,
                        ocr_text: ocr_txt,
                        caption_text: cap_txt,
                    })?;
                    self.repo
                        .upsert_document(entry, title, &body, has_body, &terms)?;
                    if self.cfg.embeddings_enabled {
                        let hash = hash_file_blake3(&entry.path).unwrap_or_default();
                        if has_body {
                            match self.embeddings.embed_document(&body) {
                                Ok(vector) => self.repo.upsert_embedding(
                                    &p_str,
                                    TEXT_MODEL_ID,
                                    "text",
                                    &vector,
                                    &hash,
                                )?,
                                Err(error) => eprintln!(
                                    "hydrogen: image text embedding failed for {p_str}: {error}"
                                ),
                            }
                        }
                        match self.embeddings.embed_image(&entry.path) {
                            Ok(vector) => self.repo.upsert_embedding(
                                &p_str,
                                CLIP_MODEL_ID,
                                "clip",
                                &vector,
                                &hash,
                            )?,
                            Err(error) => {
                                eprintln!("hydrogen: image embedding failed for {p_str}: {error}")
                            }
                        }
                    }
                    if *need_ocr {
                        if ocr_failed {
                            self.repo.fail_model_job(&p_str, "image_ocr")?;
                        } else {
                            self.repo.complete_model_job(&p_str, "image_ocr")?;
                        }
                    }
                    processed += 1;
                }

                self.repo.rebuild_sparse_index_if_needed(true)?;
                self.repo.record_worker_metric(
                    "image_sync_batch",
                    batch_started.elapsed().as_millis() as u64,
                    current_rss_bytes(),
                    batch.len(),
                )?;
                bump_revision(&self.cfg.rev_file);
            }

            Ok(processed)
        })();

        let _ = fs::remove_file(&self.cfg.img_lock_path);
        let processed = run_result?;
        Ok(json!({ "status": "done", "processed": processed }).to_string())
    }

    pub fn stats(&self) -> Result<IndexStats> {
        self.repo.stats(self.is_image_sync_running())
    }

    pub fn is_image_sync_running(&self) -> bool {
        let Ok(raw) = fs::read_to_string(&self.cfg.img_lock_path) else {
            return false;
        };
        let Ok(pid) = raw.trim().parse::<i32>() else {
            let _ = fs::remove_file(&self.cfg.img_lock_path);
            return false;
        };
        if pid > 0 && unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        let _ = fs::remove_file(&self.cfg.img_lock_path);
        false
    }

    fn spawn_background_image_sync(&self) -> bool {
        if self.is_image_sync_running() {
            return false;
        }
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        Command::new(exe)
            .args(["index", "sync-images"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .is_ok()
    }
}

pub fn write_compat_files(cache_txt: &Path, rev_file: &Path, entries: &[FileEntry]) -> Result<()> {
    let mut out = String::with_capacity(entries.len() * 48);
    for e in entries {
        out.push_str(&e.path.to_string_lossy());
        if e.is_dir {
            out.push('/');
        }
        out.push('\n');
    }
    let tmp = cache_txt.with_extension("txt.tmp");
    fs::write(&tmp, out)?;
    fs::rename(&tmp, cache_txt)?;
    bump_revision(rev_file);
    Ok(())
}

pub fn rewrite_compat_from_db(
    cache_txt: &Path,
    rev_file: &Path,
    repo: &dyn IndexRepository,
) -> Result<()> {
    let rows = repo.all_indexed_paths()?;
    let mut out = String::with_capacity(rows.len() * 48);
    for (path, is_dir) in rows {
        out.push_str(&path);
        if is_dir {
            out.push('/');
        }
        out.push('\n');
    }
    let tmp = cache_txt.with_extension("txt.tmp");
    fs::write(&tmp, out)?;
    fs::rename(&tmp, cache_txt)?;
    bump_revision(rev_file);
    Ok(())
}

pub fn bump_revision(rev_file: &Path) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let tmp = rev_file.with_extension("rev.tmp");
    if fs::write(&tmp, format!("{ts}\n")).is_ok() {
        let _ = fs::rename(&tmp, rev_file);
    }
}

/// Stream files through BLAKE3 so large PDFs and images do not need to be
/// loaded into RAM. The digest is stored separately from mtime/size metadata.
fn hash_file_blake3(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::adapters::ml::embeddings::{CLIP_MODEL_ID, TEXT_MODEL_ID};
    use crate::adapters::sqlite::repo::SqliteIndexRepository;
    use crate::application::query_service::QueryService;
    use crate::domain::document::FileEntry;
    use crate::ports::extractor::{EmbeddingProvider, OcrProvider};
    use crate::ports::repository::IndexRepository;
    use std::collections::HashMap;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct FixtureScanner(Vec<FileEntry>);
    impl FileScanner for FixtureScanner {
        fn scan(&self, _max_depth: usize, _max_results: usize) -> Vec<FileEntry> {
            self.0.clone()
        }
    }

    struct FixtureExtractor;
    impl TextExtractor for FixtureExtractor {
        fn extract_text(&self, path: &Path, _size: u64) -> String {
            std::fs::read_to_string(path).unwrap_or_default()
        }
        fn is_indexable_image(&self, path: &Path, size: u64) -> bool {
            path.extension().and_then(|e| e.to_str()) == Some("png") && size >= 10_000
        }
    }

    struct FixtureModels {
        ocr_attempts: AtomicUsize,
    }
    impl OcrProvider for FixtureModels {
        fn batch_ocr_text(&self, paths: &[String]) -> HashMap<String, String> {
            if self.ocr_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return HashMap::new();
            }
            paths
                .iter()
                .map(|p| (p.clone(), "thermal chart".into()))
                .collect()
        }
        fn ocr_boxes(&self, _image_path: &Path) -> Result<Vec<crate::domain::ocr::layout::OcrBox>> {
            Ok(Vec::new())
        }
    }
    impl CaptionProvider for FixtureModels {
        fn batch_captions(&self, paths: &[String]) -> HashMap<String, String> {
            paths
                .iter()
                .map(|p| (p.clone(), "engineering diagram".into()))
                .collect()
        }
    }
    impl EmbeddingProvider for FixtureModels {
        fn embed_document(&self, text: &str) -> Result<Vec<f32>> {
            if text.trim().is_empty() {
                return Err(crate::error::HydrogenError::InvalidInput("empty".into()));
            }
            Ok(vec![1.0, 0.0, 0.0])
        }
        fn embed_clip_text(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![1.0, 0.0, 0.0])
        }
        fn embed_image(&self, _path: &Path) -> Result<Vec<f32>> {
            Ok(vec![1.0, 0.0, 0.0])
        }
    }

    struct UnavailableEmbeddings;
    impl EmbeddingProvider for UnavailableEmbeddings {
        fn embed_document(&self, _text: &str) -> Result<Vec<f32>> {
            Err(crate::error::HydrogenError::ExternalTool(
                "embedding model unavailable".into(),
            ))
        }
        fn embed_clip_text(&self, _text: &str) -> Result<Vec<f32>> {
            Err(crate::error::HydrogenError::ExternalTool(
                "CLIP model unavailable".into(),
            ))
        }
        fn embed_image(&self, _path: &Path) -> Result<Vec<f32>> {
            Err(crate::error::HydrogenError::ExternalTool(
                "image model unavailable".into(),
            ))
        }
    }

    fn entry(path: PathBuf) -> FileEntry {
        let metadata = path.metadata().unwrap();
        FileEntry {
            path,
            mtime: metadata
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            size: metadata.len(),
            is_dir: false,
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }

    #[test]
    fn sync_indexes_documents_images_vectors_and_fuses_search() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("hydrogen-e2e-{}-{nonce}", std::process::id()));
        let state = root.join("state");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("lecture.txt"),
            "Heat transfer and thermal resistance",
        )
        .unwrap();
        std::fs::write(root.join("diagram.png"), vec![7u8; 20_000]).unwrap();

        let entries = vec![
            entry(root.join("lecture.txt")),
            entry(root.join("diagram.png")),
        ];
        let scanner = FixtureScanner(entries);
        let extractor = FixtureExtractor;
        let models = FixtureModels {
            ocr_attempts: AtomicUsize::new(0),
        };
        let cfg = AppConfig {
            state_dir: state.clone(),
            db_path: state.join("index.db"),
            cache_txt: state.join("files.txt"),
            rev_file: state.join("index.rev"),
            img_lock_path: state.join("images.pid"),
            models_dir: root.join("models"),
            search_roots: vec![root.clone()],
            ics_globs: Vec::new(),
            evolution_db_globs: Vec::new(),
            default_event_duration_mins: 60,
            embeddings_enabled: true,
            inference_threads: 2,
            image_batch_size: 2,
            pdf_page_limit: 50,
        };
        let repo = SqliteIndexRepository::open(&cfg.db_path).unwrap();
        let service =
            SyncService::new(&cfg, &repo, &scanner, &extractor, &models, &models, &models);

        let first = service.sync(false).unwrap();
        assert_eq!(first.updated, 2);
        assert_eq!(first.pending_images, 1);
        let image_sync = service.sync_images(false, 0).unwrap();
        assert!(image_sync.contains("\"processed\":1"));
        assert!(repo
            .model_job_pending(&root.join("diagram.png").to_string_lossy(), "image_ocr")
            .unwrap());
        let retry_sync = service.sync_images(false, 0).unwrap();
        assert!(retry_sync.contains("\"processed\":1"));
        assert!(!repo
            .model_job_pending(&root.join("diagram.png").to_string_lossy(), "image_ocr")
            .unwrap());

        let text_vectors = repo
            .semantic_search(&[1.0, 0.0, 0.0], TEXT_MODEL_ID, "text", 10)
            .unwrap();
        assert!(text_vectors
            .iter()
            .any(|hit| hit.path.ends_with("lecture.txt")));
        let image_vectors = repo
            .semantic_search(&[1.0, 0.0, 0.0], CLIP_MODEL_ID, "clip", 10)
            .unwrap();
        assert!(image_vectors
            .iter()
            .any(|hit| hit.path.ends_with("diagram.png")));

        let query = QueryService::new(&repo, &models);
        let hits = query.search("thermal resistance", 10, "all", true).unwrap();
        assert!(hits.iter().any(|hit| hit.path.ends_with("lecture.txt")));
        let image_hits = query.search_image(&root.join("diagram.png"), 10).unwrap();
        assert!(image_hits
            .iter()
            .any(|hit| hit.path.ends_with("diagram.png")));

        // A missing optional model must degrade to lexical retrieval, not an error.
        let unavailable = UnavailableEmbeddings;
        let lexical_fallback = QueryService::new(&repo, &unavailable);
        let fallback_hits = lexical_fallback
            .search("thermal resistance", 10, "all", true)
            .unwrap();
        assert!(fallback_hits
            .iter()
            .any(|hit| hit.path.ends_with("lecture.txt")));

        // Preserve the scanner's original size/mtime metadata while changing
        // the bytes. The BLAKE3 digest must still trigger re-indexing.
        std::fs::write(
            root.join("lecture.txt"),
            "Mass transfer and thermal resistance",
        )
        .unwrap();
        let changed = service.sync(false).unwrap();
        assert_eq!(changed.updated, 1);
        let mass_hits = repo.search("mass transfer", 10, "all").unwrap();
        assert!(mass_hits
            .iter()
            .any(|hit| hit.path.ends_with("lecture.txt")));

        let unchanged = service.sync(false).unwrap();
        assert_eq!(
            unchanged.updated, 0,
            "unchanged files should not be re-embedded"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn current_rss_bytes() -> u64 {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    status
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix("VmRSS:")?.split_whitespace().next()?;
            value.parse::<u64>().ok().map(|kib| kib * 1024)
        })
        .unwrap_or(0)
}
