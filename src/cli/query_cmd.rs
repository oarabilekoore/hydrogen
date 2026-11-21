use crate::adapters::ml::embeddings::FastEmbedProvider;
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::application::query_service::QueryService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct QueryArgs {
    /// Search query string
    pub query: Option<String>,
    /// Search visually similar images using a local image as the query.
    #[arg(long)]
    pub image: Option<PathBuf>,
    /// Maximum number of ranked hits to return
    #[arg(long)]
    pub limit: Option<usize>,
    /// Restrict results to "all", "files" (filename/path), or "content" (body/OCR/caption)
    #[arg(long, default_value = "all")]
    pub mode: String,
    /// Augment lexical search with local MiniLM and shared CLIP vectors.
    #[arg(long)]
    pub semantic: bool,
    /// Disable semantic retrieval even when it is enabled in config.
    #[arg(long)]
    pub no_semantic: bool,
}

pub fn run(args: QueryArgs, cfg: &AppConfig) -> Result<()> {
    if !cfg.db_path.exists() {
        println!("[]");
        return Ok(());
    }
    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let embeddings = FastEmbedProvider::new(cfg.models_dir.clone(), cfg.inference_threads);
    let service = QueryService::new(&repo, &embeddings);
    let limit = args.limit.unwrap_or(cfg.settings.search.default_limit);
    let semantic = !args.no_semantic && (args.semantic || cfg.settings.indexing.embeddings_enabled);
    let hits = match (args.query.as_deref(), args.image.as_deref()) {
        (Some(query), None) => service.search(query, limit, &args.mode, semantic)?,
        (None, Some(image)) => service.search_image(image, limit)?,
        (Some(_), Some(_)) => {
            return Err(crate::error::HydrogenError::InvalidInput(
                "provide either a text query or --image, not both".into(),
            ));
        }
        (None, None) => {
            return Err(crate::error::HydrogenError::InvalidInput(
                "provide a text query or --image PATH".into(),
            ));
        }
    };
    println!("{}", serde_json::to_string(&hits)?);
    Ok(())
}
