use crate::adapters::ml::embeddings::FastEmbedProvider;
use crate::adapters::sqlite::knowledge_repo::KnowledgeRepository;
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::application::query_service::QueryService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::Args;
use serde_json::json;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Search across local files, saved articles and conversations
    pub query: String,
    /// Maximum results per source category
    #[arg(long)]
    pub limit: Option<usize>,
}

pub fn run(args: SearchArgs, cfg: &AppConfig) -> Result<()> {
    let query = args.query.trim();
    if query.len() < 2 {
        println!("{}", json!({"files":[],"knowledge":[]}));
        return Ok(());
    }
    let limit = args.limit.unwrap_or(cfg.settings.search.default_limit).clamp(1, 100);
    let files = if cfg.db_path.exists() {
        let repo = SqliteIndexRepository::open(&cfg.db_path)?;
        let embeddings = FastEmbedProvider::new(cfg.models_dir.clone(), cfg.inference_threads);
        let service = QueryService::new(&repo, &embeddings);
        service.search(query, limit, "all", cfg.settings.indexing.embeddings_enabled)?
    } else {
        Vec::new()
    };
    let knowledge = KnowledgeRepository::open(&cfg.db_path)?.search_knowledge(query, limit)?;
    println!("{}", serde_json::to_string(&json!({
        "files": files,
        "knowledge": knowledge
    }))?);
    Ok(())
}
