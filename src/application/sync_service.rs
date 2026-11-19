use crate::config::AppConfig;
use crate::domain::document::{
    compose_image_body, CachedImage, FileEntry, IndexStats, SyncSummary,
};
use crate::domain::search::tfidf::extract_doc_terms;
use crate::error::Result;
use crate::ports::extractor::{CaptionProvider, FileScanner, OcrProvider, TextExtractor};
use crate::ports::repository::IndexRepository;
use serde_json::json;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct SyncService<'a> {
    cfg: &'a AppConfig,
    repo: &'a dyn IndexRepository,
    scanner: &'a dyn FileScanner,
    extractor: &'a dyn TextExtractor,
    ocr: &'a dyn OcrProvider,
    captioner: &'a dyn CaptionProvider,
}

impl<'a> SyncService<'a> {
    pub fn new(
        cfg: &'a AppConfig,
        repo: &'a dyn IndexRepository,
        scanner: &'a dyn FileScanner,
        extractor: &'a dyn TextExtractor,
        ocr: &'a dyn OcrProvider,
        captioner: &'a dyn CaptionProvider,
    ) -> Self {
        Self {
            cfg,
            repo,
            scanner,
            extractor,
            ocr,
            captioner,
        }
    }

    pub fn sync(&self, spawn_images: bool) -> Result<SyncSummary> {
        fs::create_dir_all(&self.cfg.state_dir)?;
        let entries = self.scanner.scan(6, 5000);

        let mut existing = self.repo.load_file_records()?;
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
            if let Some(old_path) = self.repo.find_by_inode(entry.dev, entry.ino)? {
                if old_path != p_str && !current_paths.contains(&old_path) {
                    if self.repo.rename_path(&old_path, &p_str, entry.is_dir)? > 0 {
                        existing.remove(&old_path);
                        if let Some(mut ic) = img_cached.remove(&old_path) {
                            ic.path = p_str.clone();
                            img_cached.insert(p_str.clone(), ic);
                        }
                        moved += 1;
                    }
                }
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
            if !entry.is_dir && prev != Some((entry.mtime, entry.size)) {
                if let Ok(hash) = hash_file_blake3(&entry.path) {
                    self.repo.upsert_content_hash(&p_str, &hash, entry.size, entry.mtime)?;
                }
                if entry.path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("pdf")).unwrap_or(false) {
                    self.repo.enqueue_model_job(&p_str, "pdf_pages")?;
                } else if self.extractor.is_indexable_image(&entry.path, entry.size) {
                    self.repo.enqueue_model_job(&p_str, "image_ocr")?;
                }
            }
            let title = entry
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&p_str);
            let is_img = !entry.is_dir && self.extractor.is_indexable_image(&entry.path, entry.size);

            if is_img {
                if let Some(ic) = img_cached.get(&p_str) {
                    if ic.mtime == entry.mtime && ic.size == entry.size {
                        if prev == Some((entry.mtime, entry.size)) {
                            continue;
                        }
                        let body = compose_image_body(&ic.caption_text, &ic.ocr_text);
                        let has_body = !body.is_empty();
                        let terms = extract_doc_terms(title, &ic.ocr_text, &ic.caption_text);
                        self.repo
                            .upsert_document(entry, title, &body, has_body, &terms)?;
                        updated += 1;
                        continue;
                    }
                }

                pending_images += 1;
                if prev == Some((entry.mtime, entry.size)) {
                    continue;
                }
                let terms = extract_doc_terms(title, "", "");
                self.repo.upsert_document(entry, title, "", false, &terms)?;
                updated += 1;
            } else {
                if prev == Some((entry.mtime, entry.size)) {
                    continue;
                }
                let body = if entry.is_dir {
                    String::new()
                } else {
                    self.extractor.extract_text(&entry.path, entry.size)
                };
                let has_body = !body.trim().is_empty();
                let terms = extract_doc_terms(title, &body, "");
                self.repo
                    .upsert_document(entry, title, &body, has_body, &terms)?;
                updated += 1;
            }
        }

        self.repo
            .rebuild_sparse_index_if_needed(updated > 0 || moved > 0 || removed > 0)?;

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
                match img_cached.get(&p_str) {
                    None => queue.push((entry, true, true)),
                    Some(ic) if ic.mtime != entry.mtime || ic.size != entry.size => {
                        queue.push((entry, true, true));
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
            for batch in queue.chunks(8) {
                let ocr_targets: Vec<String> = batch
                    .iter()
                    .filter(|(_, need_ocr, _)| *need_ocr)
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
                    let ocr_txt = if *need_ocr {
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
                    processed += 1;
                }

                self.repo.rebuild_sparse_index_if_needed(true)?;
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
        if read == 0 { break; }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}
