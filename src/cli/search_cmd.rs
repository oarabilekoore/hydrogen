use crate::adapters::ml::embeddings::FastEmbedProvider;
use crate::adapters::sqlite::knowledge::KnowledgeStore;
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::application::query_service::QueryService;
use crate::config::AppConfig;
use crate::error::{HydrogenError, Result};
use clap::Args;
use serde_json::{json, Value};

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Search files, saved articles, conversations and generated answers
    pub query: String,
    /// Maximum number of results per source group
    #[arg(long)]
    pub limit: Option<usize>,
}

pub fn run(args: SearchArgs, cfg: &AppConfig) -> Result<()> {
    let query = args.query.trim();
    if query.is_empty() {
        return Err(HydrogenError::InvalidInput("search query cannot be empty".into()));
    }
    if !cfg.db_path.exists() {
        println!("[]");
        return Ok(());
    }

    let limit = args.limit.unwrap_or(cfg.settings.search.default_limit).clamp(1, 100);
    let knowledge = KnowledgeStore::open(&cfg.db_path).map_err(|error| {
        HydrogenError::Config(format!("could not open saved knowledge: {error}"))
    })?;
    let mut results: Vec<Value> = knowledge
        .search_saved_knowledge(query, limit)?
        .into_iter()
        .map(|hit| json!({
            "kind": hit.kind,
            "id": hit.id,
            "title": hit.title,
            "snippet": hit.snippet,
            "url": hit.url,
            "updated_at": hit.updated_at
        }))
        .collect();

    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let embeddings = FastEmbedProvider::new(cfg.models_dir.clone(), cfg.inference_threads);
    let service = QueryService::new(&repo, &embeddings);
    let files = service.search(
        query,
        limit,
        "all",
        cfg.settings.indexing.embeddings_enabled,
    )?;
    results.extend(files.into_iter().map(|hit| json!({
        "kind": "file",
        "id": hit.path,
        "path": hit.path,
        "name": hit.name,
        "source": hit.source,
        "score": hit.score,
        "vec": hit.vec,
        "fts": hit.fts,
        "coverage": hit.coverage,
        "phrase": hit.phrase,
        "snippet": hit.snippet,
        "isDir": hit.is_dir,
        "page": hit.page,
        "line": hit.line,
        "preview": hit.preview
    })));

    println!("{}", serde_json::to_string(&results)?);
    Ok(())
}
