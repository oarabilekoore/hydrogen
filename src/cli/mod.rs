pub mod answer_cmd;
pub mod calendar_cmd;
pub mod config_cmd;
pub mod daemon_cmd;
pub mod doctor_cmd;
pub mod index_cmd;
pub mod ocr_cmd;
pub mod query_cmd;
pub mod server_cmd;
pub mod search_cmd;

use crate::config::AppConfig;
use crate::error::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "hydrogen",
    version,
    about = "Native indexer, file watcher, OCR, and calendar backend for Quickshell"
)]
pub struct Cli {
    /// Start the bundled Hydrogen web application and local API
    #[arg(long)]
    pub frontend: bool,
    /// Start a new frontend conversation with this question
    #[arg(long, requires = "frontend")]
    pub ask: Option<String>,
    /// Open an existing saved conversation in the frontend
    #[arg(long, requires = "frontend")]
    pub conversation: Option<i64>,
    /// Open a saved offline article in the frontend
    #[arg(long, requires = "frontend")]
    pub page: Option<i64>,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Inspect and initialise Hydrogen configuration
    Config(config_cmd::ConfigArgs),
    /// Answer a question from indexed documents using a local Ollama model
    Answer(answer_cmd::AnswerArgs),
    /// Manage the sparse TF-IDF + FTS5 + image OCR/VLM index
    Index(index_cmd::IndexArgs),
    /// Unified filename + sparse vector + FTS5 + vision search
    Query(query_cmd::QueryArgs),
    /// Run real-time Linux inotify file watcher & incremental indexer
    Daemon(daemon_cmd::DaemonArgs),
    /// Extract Markdown with Pix2Text, TrOCR, or legacy RapidOCR
    Ocr(ocr_cmd::OcrArgs),
    /// Search files, saved web pages and conversations
    Search(search_cmd::SearchArgs),
    /// Start the bundled Hydrogen web application and local API
    Frontend,
    /// Diagnose local dependencies, models, database and worker state
    Doctor,
    /// List, create, or launch desktop calendar events
    Calendar(calendar_cmd::CalendarArgs),
}

pub fn dispatch(cli: Cli, cfg: &AppConfig) -> Result<()> {
    if cli.frontend {
        return server_cmd::run(cfg, cli.ask.as_deref(), cli.conversation, cli.page);
    }
    match cli.command {
        Some(Commands::Config(args)) => config_cmd::run(args, cfg),
        Some(Commands::Answer(args)) => answer_cmd::run(args, cfg),
        Some(Commands::Index(args)) => index_cmd::run(args, cfg),
        Some(Commands::Query(args)) => query_cmd::run(args, cfg),
        Some(Commands::Daemon(args)) => daemon_cmd::run(args, cfg),
        Some(Commands::Search(args)) => search_cmd::run(args, cfg),
        Some(Commands::Frontend) => server_cmd::run(cfg, None, None, None),
        Some(Commands::Doctor) => doctor_cmd::run(cfg),
        Some(Commands::Ocr(args)) => ocr_cmd::run(args, cfg),
        Some(Commands::Calendar(args)) => calendar_cmd::run(args, cfg),
        None => Err(crate::error::HydrogenError::InvalidInput(
            "provide a command or use --frontend".into(),
        )),
    }
}
