use crate::adapters::fs::text_extract::is_plain_text_path;
use crate::domain::document::{is_image_extension, CachedImage, FileEntry, IndexStats, SearchHit};
use crate::domain::search::ranker::{
    best_pdf_page, collapse_whitespace, inspect_passage_and_phrase, locate_match_line,
    score_filename_match, similarity_ratio,
};
use crate::domain::search::tfidf::{compute_idf, normalize_doc_weights};
use crate::domain::search::tokenizer::tokenize_content;
use crate::error::Result;
use crate::ports::repository::IndexRepository;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i32 = 6;

pub struct SqliteIndexRepository {
    db_path: PathBuf,
}

impl SqliteIndexRepository {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let repo = Self {
            db_path: db_path.to_path_buf(),
        };
        let conn = repo.connect()?;
        repo.init_schema(&conn)?;
        Ok(repo)
    }

    fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(std::time::Duration::from_secs(15))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;",
        )?;
        Ok(conn)
    }

    fn init_schema(&self, conn: &Connection) -> Result<()> {
        let ver: i32 = conn.query_row("PRAGMA user_version;", [], |r| r.get(0))?;
        if ver < SCHEMA_VERSION {
            // Additive migrations preserve the user's existing index and OCR cache.
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        }

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL,
                is_dir INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS inode_map (
                path TEXT PRIMARY KEY,
                dev INTEGER NOT NULL,
                ino INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_inode_dev_ino ON inode_map(dev, ino);
            CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
                path UNINDEXED,
                title,
                body,
                tokenize='porter unicode61'
            );
            CREATE TABLE IF NOT EXISTS doc_terms (
                path TEXT NOT NULL,
                term TEXT NOT NULL,
                tf_w REAL NOT NULL,
                PRIMARY KEY (path, term)
            );
            CREATE TABLE IF NOT EXISTS term_postings (
                term TEXT NOT NULL,
                path TEXT NOT NULL,
                weight REAL NOT NULL,
                PRIMARY KEY (term, path)
            );
            CREATE TABLE IF NOT EXISTS idf_stats (
                term TEXT PRIMARY KEY,
                idf REAL NOT NULL
            );
            CREATE TABLE IF NOT EXISTS doc_meta (
                path TEXT PRIMARY KEY,
                has_body INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS image_cache (
                path TEXT PRIMARY KEY,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL,
                ocr_text TEXT NOT NULL DEFAULT '',
                caption_text TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS content_hashes (
                path TEXT PRIMARY KEY, content_hash TEXT NOT NULL,
                size INTEGER NOT NULL, mtime INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pdf_pages (
                path TEXT NOT NULL, page_no INTEGER NOT NULL,
                content_hash TEXT NOT NULL DEFAULT '', body TEXT NOT NULL DEFAULT '',
                preview_path TEXT NOT NULL DEFAULT '', PRIMARY KEY(path, page_no)
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS pdf_pages_fts USING fts5(
                path UNINDEXED, page_no UNINDEXED, body, tokenize='unicode61'
            );
            CREATE TABLE IF NOT EXISTS embedding_vectors (
                path TEXT NOT NULL, model_id TEXT NOT NULL, modality TEXT NOT NULL,
                dimensions INTEGER NOT NULL, vector BLOB NOT NULL,
                content_hash TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY(path, model_id, modality)
            );
            CREATE INDEX IF NOT EXISTS idx_embedding_model_modality
                ON embedding_vectors(model_id, modality);
            CREATE TABLE IF NOT EXISTS model_jobs (
                job_id INTEGER PRIMARY KEY, path TEXT NOT NULL, task TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued', attempts INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()), UNIQUE(path, task)
            );
            CREATE INDEX IF NOT EXISTS idx_model_jobs_status ON model_jobs(status, updated_at);",
        )?;
        Ok(())
    }

    fn rebuild_sparse_index(&self, conn: &mut Connection) -> Result<()> {
        let total_docs: usize =
            conn.query_row("SELECT COUNT(*) FROM doc_meta", [], |r| r.get(0))?;
        if total_docs == 0 {
            conn.execute("DELETE FROM idf_stats", [])?;
            conn.execute("DELETE FROM term_postings", [])?;
            return Ok(());
        }

        let mut has_body_map: HashMap<String, bool> = HashMap::with_capacity(total_docs);
        {
            let mut stmt = conn.prepare("SELECT path, has_body FROM doc_meta")?;
            let rows = stmt.query_map([], |r| {
                let p: String = r.get(0)?;
                let hb: i64 = r.get(1)?;
                Ok((p, hb != 0))
            })?;
            for row in rows {
                let (p, hb) = row?;
                has_body_map.insert(p, hb);
            }
        }

        let mut idf_map: HashMap<String, f64> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT term, COUNT(*) FROM doc_terms GROUP BY term")?;
            let rows = stmt.query_map([], |r| {
                let t: String = r.get(0)?;
                let df: usize = r.get(1)?;
                Ok((t, df))
            })?;
            for row in rows {
                let (term, df) = row?;
                idf_map.insert(term, compute_idf(total_docs, df));
            }
        }

        let mut doc_rows: Vec<(String, String, f64)> = Vec::new();
        {
            let mut stmt = conn.prepare("SELECT path, term, tf_w FROM doc_terms ORDER BY path")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            })?;
            for row in rows {
                doc_rows.push(row?);
            }
        }

        let tx = conn.transaction()?;
        tx.execute("DELETE FROM idf_stats", [])?;
        {
            let mut ins = tx.prepare("INSERT INTO idf_stats (term, idf) VALUES (?1, ?2)")?;
            for (term, idf) in &idf_map {
                ins.execute(params![term, idf])?;
            }
        }

        tx.execute("DELETE FROM term_postings", [])?;
        {
            let mut ins =
                tx.prepare("INSERT INTO term_postings (term, path, weight) VALUES (?1, ?2, ?3)")?;

            let mut cur_path: Option<String> = None;
            let mut cur_items: Vec<(String, f64)> = Vec::new();

            for (path, term, tf_w) in doc_rows {
                if cur_path.as_deref() != Some(path.as_str()) {
                    if let Some(ref prev_p) = cur_path {
                        let hb = *has_body_map.get(prev_p).unwrap_or(&false);
                        for (t, norm_w) in normalize_doc_weights(&cur_items, hb) {
                            ins.execute(params![t, prev_p, norm_w])?;
                        }
                        cur_items.clear();
                    }
                    cur_path = Some(path);
                }
                let idf = *idf_map.get(&term).unwrap_or(&1.0);
                cur_items.push((term, tf_w * idf));
            }

            if let Some(ref prev_p) = cur_path {
                let hb = *has_body_map.get(prev_p).unwrap_or(&false);
                for (t, norm_w) in normalize_doc_weights(&cur_items, hb) {
                    ins.execute(params![t, prev_p, norm_w])?;
                }
            }
        }

        tx.commit()?;
        Ok(())
    }
}

impl IndexRepository for SqliteIndexRepository {
    fn replace_pdf_pages(
        &self,
        path: &str,
        pages: &[(i64, String, String)],
        hash: &str,
    ) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM pdf_pages WHERE path=?1", params![path])?;
        tx.execute("DELETE FROM pdf_pages_fts WHERE path=?1", params![path])?;
        for (page_no, body, preview) in pages {
            tx.execute("INSERT INTO pdf_pages(path,page_no,content_hash,body,preview_path) VALUES(?1,?2,?3,?4,?5)", params![path,page_no,hash,body,preview])?;
            tx.execute(
                "INSERT INTO pdf_pages_fts(path,page_no,body) VALUES(?1,?2,?3)",
                params![path, page_no, body],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn upsert_content_hash(&self, path: &str, hash: &str, size: u64, mtime: i64) -> Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO content_hashes(path,content_hash,size,mtime) VALUES(?1,?2,?3,?4)
             ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,size=excluded.size,mtime=excluded.mtime",
            params![path, hash, size as i64, mtime],
        )?;
        Ok(())
    }

    fn content_hash(&self, path: &str) -> Result<Option<String>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT content_hash FROM content_hashes WHERE path=?1",
            params![path],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    fn enqueue_model_job(&self, path: &str, task: &str) -> Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO model_jobs(path,task,status) VALUES(?1,?2,'queued')
             ON CONFLICT(path,task) DO UPDATE SET status=CASE WHEN model_jobs.status='done' THEN 'queued' ELSE model_jobs.status END, updated_at=unixepoch()",
            params![path, task],
        )?;
        Ok(())
    }

    fn load_file_records(&self) -> Result<HashMap<String, (i64, u64)>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare("SELECT path, mtime, size FROM files")?;
        let rows = stmt.query_map([], |r| {
            let p: String = r.get(0)?;
            let mt: i64 = r.get(1)?;
            let sz: i64 = r.get(2)?;
            Ok((p, (mt, sz.max(0) as u64)))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (p, meta) = row?;
            out.insert(p, meta);
        }
        Ok(out)
    }

    fn load_image_cache(&self) -> Result<HashMap<String, CachedImage>> {
        let conn = self.connect()?;
        let mut stmt =
            conn.prepare("SELECT path, mtime, size, ocr_text, caption_text FROM image_cache")?;
        let rows = stmt.query_map([], |r| {
            let sz: i64 = r.get(2)?;
            Ok(CachedImage {
                path: r.get(0)?,
                mtime: r.get(1)?,
                size: sz.max(0) as u64,
                ocr_text: r.get(3)?,
                caption_text: r.get(4)?,
            })
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let ic = row?;
            out.insert(ic.path.clone(), ic);
        }
        Ok(out)
    }

    fn find_by_inode(&self, dev: u64, ino: u64) -> Result<Option<String>> {
        let conn = self.connect()?;
        let res = conn
            .query_row(
                "SELECT path FROM inode_map WHERE dev = ?1 AND ino = ?2 LIMIT 1",
                params![dev as i64, ino as i64],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(res)
    }

    fn upsert_document(
        &self,
        entry: &FileEntry,
        title: &str,
        body: &str,
        has_body: bool,
        terms: &[(String, f64)],
    ) -> Result<()> {
        let p_str = entry.path.to_string_lossy();
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;

        tx.execute(
            "DELETE FROM files_fts WHERE path = ?1",
            params![p_str.as_ref()],
        )?;
        tx.execute(
            "INSERT INTO files_fts (path, title, body) VALUES (?1, ?2, ?3)",
            params![p_str.as_ref(), title, body],
        )?;

        tx.execute(
            "DELETE FROM doc_terms WHERE path = ?1",
            params![p_str.as_ref()],
        )?;
        {
            let mut ins =
                tx.prepare("INSERT INTO doc_terms (path, term, tf_w) VALUES (?1, ?2, ?3)")?;
            for (t, w) in terms {
                ins.execute(params![p_str.as_ref(), t, w])?;
            }
        }

        tx.execute(
            "INSERT OR REPLACE INTO doc_meta (path, has_body) VALUES (?1, ?2)",
            params![p_str.as_ref(), if has_body { 1 } else { 0 }],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO files (path, mtime, size, is_dir) VALUES (?1, ?2, ?3, ?4)",
            params![
                p_str.as_ref(),
                entry.mtime,
                entry.size as i64,
                if entry.is_dir { 1 } else { 0 }
            ],
        )?;
        if entry.ino != 0 {
            tx.execute(
                "INSERT OR REPLACE INTO inode_map (path, dev, ino) VALUES (?1, ?2, ?3)",
                params![p_str.as_ref(), entry.dev as i64, entry.ino as i64],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    fn upsert_image_cache(&self, img: &CachedImage) -> Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT OR REPLACE INTO image_cache (path, mtime, size, ocr_text, caption_text)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                img.path,
                img.mtime,
                img.size as i64,
                img.ocr_text,
                img.caption_text
            ],
        )?;
        Ok(())
    }

    fn rename_path(&self, old_path: &str, new_path: &str, is_dir: bool) -> Result<usize> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let tables = [
            "files",
            "inode_map",
            "files_fts",
            "doc_terms",
            "term_postings",
            "doc_meta",
            "image_cache",
            "content_hashes",
            "pdf_pages",
            "pdf_pages_fts",
            "embedding_vectors",
            "model_jobs",
        ];

        let mut affected = 0usize;
        if is_dir {
            let prefix_start = (old_path.chars().count() + 1) as i64;
            for tbl in tables {
                tx.execute(
                    &format!("DELETE FROM {tbl} WHERE path = ?1 OR path LIKE ?1 || '/%'"),
                    params![new_path],
                )?;
                let n = tx.execute(
                    &format!(
                        "UPDATE {tbl} SET path = ?1 || substr(path, ?2)
                         WHERE path = ?3 OR path LIKE ?3 || '/%'"
                    ),
                    params![new_path, prefix_start, old_path],
                )?;
                if tbl == "files" {
                    affected += n;
                }
            }
        } else {
            for tbl in tables {
                tx.execute(
                    &format!("DELETE FROM {tbl} WHERE path = ?1"),
                    params![new_path],
                )?;
                let n = tx.execute(
                    &format!("UPDATE {tbl} SET path = ?1 WHERE path = ?2"),
                    params![new_path, old_path],
                )?;
                if tbl == "files" {
                    affected += n;
                }
            }
            // Mark files.mtime = -1 so `sync` refreshes filename tokens while keeping image_cache intact
            tx.execute(
                "UPDATE files SET mtime = -1 WHERE path = ?1",
                params![new_path],
            )?;
        }

        tx.commit()?;
        Ok(affected)
    }

    fn delete_path(&self, target: &str, is_dir: bool) -> Result<usize> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let tables = [
            "files",
            "inode_map",
            "files_fts",
            "doc_terms",
            "term_postings",
            "doc_meta",
            "image_cache",
            "content_hashes",
            "pdf_pages",
            "pdf_pages_fts",
            "embedding_vectors",
            "model_jobs",
        ];

        let mut deleted = 0usize;
        for tbl in tables {
            let n = if is_dir {
                tx.execute(
                    &format!("DELETE FROM {tbl} WHERE path = ?1 OR path LIKE ?1 || '/%'"),
                    params![target],
                )?
            } else {
                tx.execute(
                    &format!("DELETE FROM {tbl} WHERE path = ?1"),
                    params![target],
                )?
            };
            if tbl == "files" {
                deleted += n;
            }
        }
        tx.commit()?;
        if deleted > 0 {
            self.rebuild_sparse_index_if_needed(true)?;
        }
        Ok(deleted)
    }

    fn remove_paths(&self, paths: &[String]) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let tables = [
            "files",
            "inode_map",
            "files_fts",
            "doc_terms",
            "term_postings",
            "doc_meta",
            "image_cache",
            "content_hashes",
            "pdf_pages",
            "pdf_pages_fts",
            "embedding_vectors",
            "model_jobs",
        ];
        for tbl in tables {
            let mut stmt = tx.prepare(&format!("DELETE FROM {tbl} WHERE path = ?1"))?;
            for p in paths {
                stmt.execute(params![p])?;
            }
        }
        tx.commit()?;
        if !paths.is_empty() {
            self.rebuild_sparse_index_if_needed(true)?;
        }
        Ok(())
    }

    fn rebuild_sparse_index_if_needed(&self, force: bool) -> Result<()> {
        let mut conn = self.connect()?;
        let postings_count: usize =
            conn.query_row("SELECT COUNT(*) FROM term_postings", [], |r| r.get(0))?;
        let files_count: usize = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;

        if force || (files_count > 0 && postings_count == 0) {
            self.rebuild_sparse_index(&mut conn)?;
        }
        Ok(())
    }

    fn search(&self, query: &str, limit: usize, mode: &str) -> Result<Vec<SearchHit>> {
        let conn = self.connect()?;
        let q_tokens = tokenize_content(query);
        let q_lower = query.to_ascii_lowercase();

        // Build directory map & filename match scores
        let mut dir_map: HashMap<String, bool> = HashMap::new();
        let mut fname_scores: HashMap<String, f64> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT path, is_dir FROM files")?;
            let rows = stmt.query_map([], |r| {
                let p: String = r.get(0)?;
                let d: i64 = r.get(1)?;
                Ok((p, d != 0))
            })?;
            for row in rows {
                let (p, is_dir) = row?;
                let name = Path::new(&p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&p);
                let f_score = score_filename_match(&p, name, &q_lower, &q_tokens);
                if f_score > 0.0 {
                    fname_scores.insert(p.clone(), f_score);
                }
                dir_map.insert(p, is_dir);
            }
        }

        if mode == "files" {
            let mut hits: Vec<SearchHit> = fname_scores
                .into_iter()
                .map(|(path, score)| {
                    let is_dir = *dir_map.get(&path).unwrap_or(&false);
                    let name = Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&path)
                        .to_string();
                    let source = if is_dir {
                        "dir"
                    } else if is_image_extension(Path::new(&path)) {
                        "image"
                    } else {
                        "file"
                    };
                    SearchHit {
                        path,
                        name,
                        source: source.to_string(),
                        score,
                        vec: 0.0,
                        fts: 0.0,
                        coverage: 1.0,
                        phrase: false,
                        snippet: String::new(),
                        is_dir,
                        page: None,
                        line: None,
                        preview: String::new(),
                    }
                })
                .collect();
            hits.sort_by(|a, b| b.score.total_cmp(&a.score));
            hits.truncate(limit);
            return Ok(hits);
        }

        // Sparse TF-IDF query expansion + typo/prefix recovery
        let mut raw_weights: HashMap<String, f64> = HashMap::new();
        let mut term_slots: HashMap<String, HashSet<String>> = HashMap::new();
        let mut seen_toks = HashSet::new();
        let unique_toks: Vec<String> = q_tokens
            .iter()
            .filter(|t| seen_toks.insert((*t).clone()))
            .cloned()
            .collect();

        for tok in &unique_toks {
            let key = format!("w:{tok}");
            let exact: Option<f64> = conn
                .query_row(
                    "SELECT idf FROM idf_stats WHERE term = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?;

            if let Some(idf) = exact {
                raw_weights.insert(key.clone(), idf);
                term_slots.entry(key).or_default().insert(tok.clone());
            } else if tok.chars().count() >= 4 {
                let prefix: String = tok.chars().take(2).collect();
                let mut stmt =
                    conn.prepare("SELECT term, idf FROM idf_stats WHERE term LIKE ?1 LIMIT 400")?;
                let rows = stmt.query_map(params![format!("w:{prefix}%")], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
                })?;

                let mut scored_cands: Vec<(f64, String, f64)> = Vec::new();
                for row in rows {
                    let (cand_term, idf) = row?;
                    let cand_word = cand_term.strip_prefix("w:").unwrap_or(&cand_term);
                    let sim = if cand_word.starts_with(tok) {
                        0.92
                    } else if cand_word.chars().count().abs_diff(tok.chars().count()) > 3 {
                        continue;
                    } else {
                        similarity_ratio(tok, cand_word)
                    };
                    if sim >= 0.78 {
                        scored_cands.push((sim, cand_term, idf));
                    }
                }
                scored_cands.sort_by(|a, b| b.0.total_cmp(&a.0));
                for (sim, cand_term, idf) in scored_cands.into_iter().take(3) {
                    let w = idf * 0.85 * sim;
                    let entry = raw_weights.entry(cand_term.clone()).or_insert(0.0);
                    if w > *entry {
                        *entry = w;
                    }
                    term_slots.entry(cand_term).or_default().insert(tok.clone());
                }
            }
        }

        for pair in q_tokens.windows(2) {
            let b_key = format!("b:{}_{}", pair[0], pair[1]);
            if let Some(idf) = conn
                .query_row(
                    "SELECT idf FROM idf_stats WHERE term = ?1",
                    params![b_key],
                    |r| r.get::<_, f64>(0),
                )
                .optional()?
            {
                raw_weights.insert(b_key, idf * 1.35);
            }
        }

        let norm: f64 = raw_weights.values().map(|w| w * w).sum::<f64>().sqrt();
        if norm > 1e-12 {
            let inv = 1.0 / norm;
            for w in raw_weights.values_mut() {
                *w *= inv;
            }
        }

        let mut cos_scores: HashMap<String, f64> = HashMap::new();
        let mut matched_slots: HashMap<String, HashSet<String>> = HashMap::new();

        for (term, q_w) in &raw_weights {
            let mut stmt =
                conn.prepare("SELECT path, weight FROM term_postings WHERE term = ?1")?;
            let rows = stmt.query_map(params![term], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })?;
            for row in rows {
                let (path, doc_w) = row?;
                *cos_scores.entry(path.clone()).or_insert(0.0) += q_w * doc_w;
                if let Some(slots) = term_slots.get(term) {
                    matched_slots
                        .entry(path)
                        .or_default()
                        .extend(slots.iter().cloned());
                }
            }
        }

        // FTS5 BM25 retrieval
        let mut fts_hits: HashMap<String, (f64, String, bool)> = HashMap::new();
        let quoted: Vec<String> = unique_toks
            .iter()
            .take(10)
            .map(|t| format!("\"{t}\"*"))
            .collect();

        if !quoted.is_empty() {
            let mut fts_queries = vec![(quoted.join(" AND "), true)];
            if quoted.len() > 1 {
                fts_queries.push((quoted.join(" OR "), false));
            }

            for (fts_q, is_and) in fts_queries {
                if !is_and && fts_hits.len() >= 5 {
                    break;
                }
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT path,
                            bm25(files_fts, 10.0, 1.0) AS rank,
                            snippet(files_fts, 2, '', '', '…', 18) AS snip
                     FROM files_fts
                     WHERE files_fts MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                ) {
                    if let Ok(rows) = stmt.query_map(params![fts_q, (limit * 3) as i64], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, f64>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    }) {
                        for row in rows.flatten() {
                            let (path, rank, snip) = row;
                            let bm25_pos = (-rank).max(0.0);
                            let clean_snip = collapse_whitespace(snip.as_deref().unwrap_or(""));
                            let update = match fts_hits.get(&path) {
                                None => true,
                                Some((_, _, prev_and)) if is_and && !*prev_and => true,
                                Some((prev_pos, _, _)) if bm25_pos > *prev_pos => true,
                                _ => false,
                            };
                            if update {
                                fts_hits.insert(path.clone(), (bm25_pos, clean_snip, is_and));
                            }
                            if is_and {
                                matched_slots
                                    .entry(path)
                                    .or_default()
                                    .extend(unique_toks.iter().take(10).cloned());
                            }
                        }
                    }
                }
            }
        }

        let total_slots = unique_toks.len().max(1) as f64;
        let mut candidates: HashSet<String> = HashSet::new();
        candidates.extend(cos_scores.keys().cloned());
        candidates.extend(fts_hits.keys().cloned());
        if mode != "content" {
            candidates.extend(fname_scores.keys().cloned());
        }

        let mut scored: Vec<SearchHit> = Vec::new();
        for path in candidates {
            let cos_sim = *cos_scores.get(&path).unwrap_or(&0.0);
            let fts_data = fts_hits.get(&path);
            let f_bonus = if mode == "content" {
                0.0
            } else {
                *fname_scores.get(&path).unwrap_or(&0.0)
            };

            if cos_sim < 0.04 && fts_data.is_none() && f_bonus <= 0.0 {
                continue;
            }

            let mut n_matched = matched_slots.get(&path).map(|s| s.len()).unwrap_or(0);
            if n_matched == 0 && (fts_data.is_some() || f_bonus > 0.0) {
                n_matched = 1;
            }

            let coverage = (n_matched as f64 / total_slots).min(1.0);
            let cov_mult = coverage * coverage;

            let vec_points = cos_sim.max(0.0) * 1200.0;
            let fts_points = fts_data
                .map(|(bm25, _, _)| (bm25 * 30.0).min(300.0))
                .unwrap_or(0.0);
            let and_bonus = if fts_data.map(|(_, _, a)| *a).unwrap_or(false) && total_slots > 1.0 {
                250.0
            } else {
                0.0
            };

            let total = (vec_points + fts_points + and_bonus) * cov_mult + f_bonus;
            let is_dir = *dir_map.get(&path).unwrap_or(&false);
            let is_img = !is_dir && is_image_extension(Path::new(&path));
            let name = Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&path)
                .to_string();

            let source = if is_dir {
                "dir"
            } else if is_img {
                "image"
            } else if cos_sim >= 0.04 || fts_data.is_some() {
                "doc"
            } else {
                "file"
            };

            scored.push(SearchHit {
                path,
                name,
                source: source.to_string(),
                score: (total * 100.0).round() / 100.0,
                vec: (cos_sim * 1000.0).round() / 1000.0,
                fts: fts_data
                    .map(|(b, _, _)| (b * 100.0).round() / 100.0)
                    .unwrap_or(0.0),
                coverage: (coverage * 100.0).round() / 100.0,
                phrase: false,
                snippet: fts_data.map(|(_, s, _)| s.clone()).unwrap_or_default(),
                is_dir,
                page: None,
                line: None,
                preview: String::new(),
            });
        }

        scored.sort_by(|a, b| b.score.total_cmp(&a.score));
        scored.truncate(35);

        let raw_query_norm = collapse_whitespace(&q_lower);
        for item in &mut scored {
            if let Ok(Some((title, body))) = conn
                .query_row(
                    "SELECT title, body FROM files_fts WHERE path = ?1",
                    params![item.path],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()
            {
                let (phrase_hit, best_snip) = inspect_passage_and_phrase(
                    &title,
                    &body,
                    &raw_query_norm,
                    &q_tokens,
                    &item.snippet,
                );
                item.phrase = phrase_hit;
                item.snippet = best_snip;
                if phrase_hit {
                    item.score = ((item.score + 750.0) * 100.0).round() / 100.0;
                }
                if is_plain_text_path(Path::new(&item.path)) {
                    item.line =
                        locate_match_line(&body, &raw_query_norm, &q_tokens).map(|n| n as i64);
                }
            }

            if item.path.to_ascii_lowercase().ends_with(".pdf") {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT page_no, body, preview_path FROM pdf_pages WHERE path = ?1 ORDER BY page_no",
                ) {
                    if let Ok(rows) = stmt.query_map(params![item.path], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    }) {
                        let pages: Vec<(i64, String, String)> = rows.flatten().collect();
                        if let Some(i) = best_pdf_page(&pages, &raw_query_norm, &q_tokens) {
                            let (page_no, page_body, preview) = &pages[i];
                            item.page = Some(*page_no);
                            if !preview.is_empty() {
                                item.preview = preview.clone();
                            }
                            let (page_phrase, page_snip) = inspect_passage_and_phrase(
                                "",
                                page_body,
                                &raw_query_norm,
                                &q_tokens,
                                "",
                            );
                            if !page_snip.is_empty() {
                                item.snippet = page_snip;
                            }
                            item.phrase = item.phrase || page_phrase;
                        }
                    }
                }
            }
        }

        scored.sort_by(|a, b| b.score.total_cmp(&a.score));
        scored.truncate(limit);
        Ok(scored)
    }

    fn stats(&self, image_sync_running: bool) -> Result<IndexStats> {
        if !self.db_path.exists() {
            return Ok(IndexStats {
                indexed_files: 0,
                indexed_images: 0,
                captioned_images: 0,
                ocr_images: 0,
                image_sync_running,
                posting_rows: 0,
                idf_terms: 0,
                db_bytes: 0,
            });
        }
        let conn = self.connect()?;
        let indexed_files: usize =
            conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let indexed_images: usize =
            conn.query_row("SELECT COUNT(*) FROM image_cache", [], |r| r.get(0))?;
        let captioned_images: usize = conn.query_row(
            "SELECT COUNT(*) FROM image_cache WHERE LENGTH(TRIM(caption_text)) > 0",
            [],
            |r| r.get(0),
        )?;
        let ocr_images: usize = conn.query_row(
            "SELECT COUNT(*) FROM image_cache WHERE LENGTH(TRIM(ocr_text)) > 0",
            [],
            |r| r.get(0),
        )?;
        let posting_rows: usize =
            conn.query_row("SELECT COUNT(*) FROM term_postings", [], |r| r.get(0))?;
        let idf_terms: usize =
            conn.query_row("SELECT COUNT(*) FROM idf_stats", [], |r| r.get(0))?;
        let db_bytes = fs::metadata(&self.db_path).map(|m| m.len()).unwrap_or(0);

        Ok(IndexStats {
            indexed_files,
            indexed_images,
            captioned_images,
            ocr_images,
            image_sync_running,
            posting_rows,
            idf_terms,
            db_bytes,
        })
    }

    fn all_indexed_paths(&self) -> Result<Vec<(String, bool)>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare("SELECT path, is_dir FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            let p: String = r.get(0)?;
            let d: i64 = r.get(1)?;
            Ok((p, d != 0))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}
