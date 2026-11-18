use crate::domain::document::FileEntry;
use crate::ports::extractor::FileScanner;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
    "target",
    "build",
    ".cache",
    "dist",
    ".next",
    ".cargo",
];

pub fn should_skip_name(name: &str) -> bool {
    name.is_empty() || name.starts_with('.') || SKIP_DIRS.contains(&name)
}

pub struct WalkDirScanner {
    roots: Vec<PathBuf>,
}

impl WalkDirScanner {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }
}

impl FileScanner for WalkDirScanner {
    fn scan(&self, max_depth: usize, max_results: usize) -> Vec<FileEntry> {
        let mut found = Vec::new();

        for root in &self.roots {
            if !root.exists() {
                continue;
            }
            let iter = WalkDir::new(root)
                .max_depth(max_depth)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    if e.depth() == 0 {
                        return true;
                    }
                    e.file_name()
                        .to_str()
                        .map(|n| !should_skip_name(n))
                        .unwrap_or(false)
                });

            for entry in iter.flatten() {
                if entry.depth() == 0 {
                    continue;
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
                    .unwrap_or(0);
                let is_dir = meta.is_dir();

                found.push(FileEntry {
                    path: entry.into_path(),
                    mtime,
                    size: if is_dir { 0 } else { meta.len() },
                    is_dir,
                    dev: meta.dev(),
                    ino: meta.ino(),
                });

                if found.len() >= max_results {
                    return found;
                }
            }
        }

        found
    }
}
