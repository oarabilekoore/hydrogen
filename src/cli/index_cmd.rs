use crate::adapters::fs::scanner::WalkDirScanner;
use crate::adapters::fs::text_extract::CompositeTextExtractor;
use crate::adapters::ml::workers::{CaptionWorker, RapidOcrWorker};
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::application::sync_service::SyncService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct IndexArgs {
    #[command(subcommand)]
    pub action: IndexAction,
}

#[derive(Subcommand, Debug)]
pub enum IndexAction {
    /// Fast incremental file/doc sync + inode move recovery
    Sync {
        /// Skip spawning background image OCR/VLM worker
        #[arg(long)]
        no_images: bool,
    },
    /// Run offline RapidOCR + VLM caption indexing on eligible images
    SyncImages {
        /// Generate VLM captions for images that only have OCR so far
        #[arg(long)]
        retag: bool,
        /// Max images to process in this run (0 = all)
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// Show index statistics as JSON
    Stats,
}

pub fn run(args: IndexArgs, cfg: &AppConfig) -> Result<()> {
    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let scanner = WalkDirScanner::new(cfg.search_roots.clone());
    let extractor = CompositeTextExtractor;
    let ocr = RapidOcrWorker::new(cfg.rapidocr_python.clone());
    let captioner = CaptionWorker::new(cfg.caption_python.clone(), cfg.caption_script.clone());

    let service = SyncService::new(cfg, &repo, &scanner, &extractor, &ocr, &captioner);

    match args.action {
        IndexAction::Sync { no_images } => {
            let summary = service.sync(!no_images)?;
            println!("{}", serde_json::to_string(&summary)?);
        }
        IndexAction::SyncImages { retag, limit } => {
            let result = service.sync_images(retag, limit)?;
            println!("{result}");
        }
        IndexAction::Stats => {
            let stats = service.stats()?;
            println!("{}", serde_json::to_string(&stats)?);
        }
    }
    Ok(())
}
