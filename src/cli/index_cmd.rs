use crate::adapters::fs::scanner::WalkDirScanner;
use crate::adapters::fs::text_extract::CompositeTextExtractor;
use crate::adapters::ml::embeddings::{FastEmbedProvider, CLIP_MODEL_ID, TEXT_MODEL_ID};
use crate::adapters::ml::rust_ocr::RustOcrProvider;
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::application::sync_service::SyncService;
use crate::config::AppConfig;
use crate::domain::document::is_image_extension;
use crate::error::Result;
use crate::ports::extractor::{EmbeddingProvider, TextExtractor};
use crate::ports::repository::IndexRepository;
use clap::{Args, Subcommand};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Args, Debug)]
pub struct IndexArgs {
    #[command(subcommand)]
    pub action: IndexAction,
}

#[derive(Subcommand, Debug)]
pub enum IndexAction {
    /// Fast incremental file/doc sync + inode move recovery
    Sync {
        #[arg(long)]
        no_images: bool,
    },
    /// Run local Rust OCR indexing on eligible images
    SyncImages {
        #[arg(long)]
        retag: bool,
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// Backfill local text and CLIP image vectors for existing indexed content
    Embed {
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// Show index statistics as JSON
    Stats,
}

pub fn run(args: IndexArgs, cfg: &AppConfig) -> Result<()> {
    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let scanner = WalkDirScanner::new(
        cfg.search_roots.clone(),
        cfg.settings.search.excluded_directories.clone(),
        cfg.settings.search.excluded_extensions.clone(),
        cfg.settings.search.include_hidden,
    );
    let extractor = CompositeTextExtractor {
        images: cfg.settings.indexing.images.clone(),
    };
    let ml = RustOcrProvider::new(
        cfg.models_dir.clone(),
        cfg.inference_threads,
        cfg.settings.indexing.images.captions,
        cfg.settings.models.ocr_printed.clone().into(),
        cfg.settings.models.ocr_formula.clone().into(),
    );
    let embeddings = FastEmbedProvider::new(cfg.models_dir.clone(), cfg.inference_threads);
    let service = SyncService::new(cfg, &repo, &scanner, &extractor, &ml, &ml, &embeddings);
    match args.action {
        IndexAction::Sync { no_images } => {
            let summary = service.sync(!no_images && cfg.settings.indexing.images.enabled)?;
            println!("{}", serde_json::to_string(&summary)?);
        }
        IndexAction::SyncImages { retag, limit } => {
            let result = service.sync_images(retag, limit)?;
            println!("{result}");
        }
        IndexAction::Embed { limit } => {
            let (documents, images, failed) =
                embed_existing(&repo, &extractor, &embeddings, limit)?;
            println!(
                "{}",
                serde_json::json!({
                    "status": "done",
                    "documents": documents,
                    "images": images,
                    "failed": failed
                })
            );
        }
        IndexAction::Stats => {
            let stats = service.stats()?;
            println!("{}", serde_json::to_string(&stats)?);
        }
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<String> {
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

fn embed_existing(
    repo: &dyn IndexRepository,
    extractor: &dyn TextExtractor,
    embeddings: &dyn EmbeddingProvider,
    limit: usize,
) -> Result<(usize, usize, usize)> {
    let mut documents = 0usize;
    let mut images = 0usize;
    let mut failed = 0usize;
    let mut after = String::new();
    loop {
        if limit > 0 && documents + images >= limit {
            break;
        }
        let remaining = if limit > 0 {
            (limit - documents - images).min(32)
        } else {
            32
        };
        let batch = repo.load_embedding_batch(&after, remaining)?;
        if batch.is_empty() {
            break;
        }
        for (path, body, hash) in &batch {
            after.clone_from(path);
            if repo.has_embedding(path, TEXT_MODEL_ID, "text")? {
                continue;
            }
            match embeddings.embed_document(body) {
                Ok(vector) => {
                    repo.upsert_embedding(path, TEXT_MODEL_ID, "text", &vector, hash)?;
                    documents += 1;
                }
                Err(error) => {
                    eprintln!("gsearch: text embedding failed for {path}: {error}");
                    failed += 1;
                }
            }
            if limit > 0 && documents + images >= limit {
                break;
            }
        }
        if batch.len() < remaining {
            break;
        }
    }

    if limit == 0 || documents + images < limit {
        for (path, is_dir) in repo.all_indexed_paths()? {
            if is_dir || !is_image_extension(Path::new(&path)) {
                continue;
            }
            if limit > 0 && documents + images >= limit {
                break;
            }
            let image_path = Path::new(&path);
            let Ok(metadata) = image_path.metadata() else {
                continue;
            };
            if !extractor.is_indexable_image(image_path, metadata.len())
                || repo.has_embedding(&path, CLIP_MODEL_ID, "clip")?
            {
                continue;
            }
            match embeddings.embed_image(image_path) {
                Ok(vector) => {
                    let hash = file_hash(image_path)?;
                    repo.upsert_embedding(&path, CLIP_MODEL_ID, "clip", &vector, &hash)?;
                    images += 1;
                }
                Err(error) => {
                    eprintln!("gsearch: image embedding failed for {path}: {error}");
                    failed += 1;
                }
            }
        }
    }
    Ok((documents, images, failed))
}
