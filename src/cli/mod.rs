pub mod answer_cmd;
pub mod calendar_cmd;
pub mod config_cmd;
pub mod daemon_cmd;
pub mod doctor_cmd;
pub mod index_cmd;
pub mod ocr_cmd;
pub mod query_cmd;
pub mod serve_cmd;

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
    /// Open the bundled Hydrogen web application in your browser
    #[arg(long)]
    pub frontend: bool,
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
    /// Diagnose local dependencies, models, database and worker state
    Doctor,
    /// List, create, or launch desktop calendar events
    Calendar(calendar_cmd::CalendarArgs),
    /// Run the local Hydrogen web application service
    Serve,
}

pub fn dispatch(cli: Cli, cfg: &AppConfig) -> Result<()> {
    if cli.frontend {
        return serve_cmd::open_frontend(cfg);
    }
    match cli.command {
        Some(Commands::Config(args)) => config_cmd::run(args, cfg),
        Some(Commands::Answer(args)) => answer_cmd::run(args, cfg),
        Some(Commands::Index(args)) => index_cmd::run(args, cfg),
        Some(Commands::Query(args)) => query_cmd::run(args, cfg),
        Some(Commands::Daemon(args)) => daemon_cmd::run(args, cfg),
        Some(Commands::Doctor) => doctor_cmd::run(cfg),
        Some(Commands::Ocr(args)) => ocr_cmd::run(args, cfg),
        Some(Commands::Calendar(args)) => calendar_cmd::run(args, cfg),
        Some(Commands::Serve) => serve_cmd::run(cfg),
        None => Err(crate::error::HydrogenError::InvalidInput(
            "choose a command or use --frontend".into(),
        )),
    }
}
