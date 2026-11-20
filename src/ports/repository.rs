use crate::domain::document::{CachedImage, FileEntry, IndexStats, SearchHit};
use crate::error::Result;
use std::collections::HashMap;

pub trait IndexRepository {
    fn upsert_content_hash(&self, path: &str, hash: &str, size: u64, mtime: i64) -> Result<()>;
    fn load_content_hashes(&self) -> Result<HashMap<String, String>>;
    fn upsert_embedding(&self, path: &str, model_id: &str, modality: &str, vector: &[f32], hash: &str) -> Result<()>;
    fn has_embedding(&self, path: &str, model_id: &str, modality: &str) -> Result<bool>;\n    fn load_embedding_batch(&self, after_path: &str, limit: usize) -> Result<Vec<(String, String, String)>>;
    fn semantic_search(&self, vector: &[f32], model_id: &str, modality: &str, limit: usize) -> Result<Vec<SearchHit>>;
    fn replace_pdf_pages(
        &self,
        path: &str,
        pages: &[(i64, String, String)],
        hash: &str,
    ) -> Result<()>;
    fn enqueue_model_job(&self, path: &str, task: &str) -> Result<()>;
    fn claim_model_job(&self, path: &str, task: &str) -> Result<bool>;
    fn complete_model_job(&self, path: &str, task: &str) -> Result<()>;
    fn fail_model_job(&self, path: &str, task: &str) -> Result<()>;
    fn load_file_records(&self) -> Result<HashMap<String, (i64, u64)>>;
    fn load_image_cache(&self) -> Result<HashMap<String, CachedImage>>;
    fn find_by_inode(&self, dev: u64, ino: u64) -> Result<Option<String>>;
    fn upsert_document(
        &self,
        entry: &FileEntry,
        title: &str,
        body: &str,
        has_body: bool,
        terms: &[(String, f64)],
    ) -> Result<()>;
    fn upsert_image_cache(&self, img: &CachedImage) -> Result<()>;
    fn rename_path(&self, old_path: &str, new_path: &str, is_dir: bool) -> Result<usize>;
    fn delete_path(&self, target: &str, is_dir: bool) -> Result<usize>;
    fn remove_paths(&self, paths: &[String]) -> Result<()>;
    fn rebuild_sparse_index_if_needed(&self, force: bool) -> Result<()>;
    fn search(&self, query: &str, limit: usize, mode: &str) -> Result<Vec<SearchHit>>;
    fn stats(&self, image_sync_running: bool) -> Result<IndexStats>;
    fn all_indexed_paths(&self) -> Result<Vec<(String, bool)>>;
}
