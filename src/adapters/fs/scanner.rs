use crate::domain::document::FileEntry;
use crate::ports::extractor::FileScanner;
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct WalkDirScanner {
    roots: Vec<PathBuf>,
    excluded_directories: HashSet<String>,
    excluded_extensions: HashSet<String>,
    include_hidden: bool,
}

impl WalkDirScanner {
    pub fn new(
        roots: Vec<PathBuf>,
        excluded_directories: Vec<String>,
        excluded_extensions: Vec<String>,
        include_hidden: bool,
    ) -> Self {
        Self {
            roots,
            excluded_directories: excluded_directories.into_iter().collect(),
            excluded_extensions: excluded_extensions
                .into_iter()
                .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
                .collect(),
            include_hidden,
        }
    }

    fn should_skip_name(&self, name: &str) -> bool {
        name.is_empty() || (!self.include_hidden && name.starts_with('.')) || self.excluded_directories.contains(name)
    }
}

impl FileScanner for WalkDirScanner {
    fn scan(&self, max_depth: usize, max_results: usize) -> Vec<FileEntry> {
        let mut found = Vec::new();
        for root in &self.roots {
            if !root.exists() { continue; }
            let iter = WalkDir::new(root)
                .max_depth(max_depth)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    if entry.depth() == 0 { return true; }
                    entry.file_name().to_str()
                        .map(|name| !self.should_skip_name(name))
                        .unwrap_or(false)
                });

            for entry in iter.flatten() {
                if entry.depth() == 0 { continue; }
                let Ok(meta) = entry.metadata() else { continue; };
                let is_dir = meta.is_dir();
                if !is_dir {
                    let excluded = entry.path().extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| self.excluded_extensions.contains(&ext.to_ascii_lowercase()))
                        .unwrap_or(false);
                    if excluded { continue; }
                }
                let mtime = meta.modified().ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
                    .unwrap_or(0);
                found.push(FileEntry {
                    path: entry.into_path(),
                    mtime,
                    size: if is_dir { 0 } else { meta.len() },
                    is_dir,
                    dev: meta.dev(),
                    ino: meta.ino(),
                });
                if found.len() >= max_results { return found; }
            }
        }
        found
    }
}
