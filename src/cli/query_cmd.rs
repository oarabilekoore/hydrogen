use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::adapters::ml::embeddings::FastEmbedProvider;
use crate::application::query_service::QueryService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::Args;

#[derive(Args, Debug)]
pub struct QueryArgs {
    /// Search query string
    pub query: String,
    /// Maximum number of ranked hits to return
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    /// Restrict results to "all", "files" (filename/path), or "content" (body/OCR/caption)
    #[arg(long, default_value = "all")]
    pub mode: String,
    /// Augment lexical search with local MiniLM and shared CLIP vectors.
    #[arg(long)]
    pub semantic: bool,
}

pub fn run(args: QueryArgs, cfg: &AppConfig) -> Result<()> {
    if !cfg.db_path.exists() {
        println!("[]");
        return Ok(());
    }
    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let embeddings = FastEmbedProvider::new(cfg.models_dir.clone());
    let service = QueryService::new(&repo, &embeddings);
    let hits = service.search(&args.query, args.limit, &args.mode, args.semantic)?;
    println!("{}", serde_json::to_string(&hits)?);
    Ok(())
}
