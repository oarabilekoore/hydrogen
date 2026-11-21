use crate::adapters::fs::inotify::LinuxInotifyWatcher;
use crate::adapters::fs::scanner::WalkDirScanner;
use crate::adapters::fs::text_extract::CompositeTextExtractor;
use crate::adapters::ml::embeddings::FastEmbedProvider;
use crate::adapters::ml::rust_ocr::RustOcrProvider;
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::application::sync_service::SyncService;
use crate::application::watcher_service::WatcherService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::Args;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Args, Debug)]
pub struct DaemonArgs {
    #[arg(long)]
    pub no_images: bool,
}

pub fn run(args: DaemonArgs, cfg: &AppConfig) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r_flag = Arc::clone(&running);
    let _ = ctrlc::set_handler(move || r_flag.store(false, Ordering::SeqCst));

    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let scanner = WalkDirScanner::new(cfg.search_roots.clone(), cfg.settings.search.excluded_directories.clone(), cfg.settings.search.excluded_extensions.clone());
    let extractor = CompositeTextExtractor { images: cfg.settings.indexing.images.clone() };
    let ml = RustOcrProvider::new(cfg.models_dir.clone(), cfg.inference_threads, cfg.settings.indexing.images.captions);
    let embeddings = FastEmbedProvider::new(cfg.models_dir.clone(), cfg.inference_threads);
    let sync_service = SyncService::new(cfg, &repo, &scanner, &extractor, &ml, &ml, &embeddings);
    let mut watcher = LinuxInotifyWatcher::new(cfg.search_roots.clone())?;
    let mut daemon = WatcherService::new(cfg, &repo, &sync_service, &mut watcher, !args.no_images && cfg.settings.indexing.images.enabled);
    daemon.serve_forever(running)
}
