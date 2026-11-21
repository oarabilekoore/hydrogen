use crate::adapters::ml::embeddings::FastEmbedProvider;
use crate::adapters::sqlite::knowledge::KnowledgeStore;
use crate::adapters::sqlite::repo::SqliteIndexRepository;
use crate::application::query_service::QueryService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::Args;
use serde_json::json;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Search files, captured web pages and saved conversations
    pub query: String,
    /// Maximum number of results per knowledge type
    #[arg(long)]
    pub limit: Option<usize>,
}

pub fn run(args: SearchArgs, cfg: &AppConfig) -> Result<()> {
    let query = args.query.trim();
    if query.is_empty() || !cfg.db_path.exists() {
        println!("[]");
        return Ok(());
    }

    let limit = args.limit.unwrap_or(cfg.settings.search.default_limit).clamp(1, 100);
    let mut results = Vec::new();

    let repo = SqliteIndexRepository::open(&cfg.db_path)?;
    let embeddings = FastEmbedProvider::new(cfg.models_dir.clone(), cfg.inference_threads);
    let service = QueryService::new(&repo, &embeddings);
    let files = service.search(
        query,
        limit,
        "all",
        cfg.settings.indexing.embeddings_enabled,
    )?;
    for hit in files {
        if hit.is_dir {
            continue;
        }
        results.push(json!({
            "kind": "file",
            "id": hit.path,
            "title": hit.name,
            "path": hit.path,
            "snippet": hit.snippet,
            "page": hit.page,
            "line": hit.line,
            "score": hit.score,
            "phrase": hit.phrase,
            "source": hit.source
        }));
    }

    let knowledge = KnowledgeStore::open(&cfg.db_path).map_err(|error| {
        crate::error::HydrogenError::Config(format!("could not open saved knowledge: {error}"))
    })?;
    for hit in knowledge.search_saved_knowledge(query, limit)? {
        results.push(json!({
            "kind": hit.kind,
            "id": hit.id,
            "title": hit.title,
            "snippet": hit.snippet,
            "url": hit.url,
            "updated_at": hit.updated_at
        }));
    }

    println!("{}", serde_json::to_string(&results)?);
    Ok(())
}
