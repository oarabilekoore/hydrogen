pub mod answer_cmd;
pub mod calendar_cmd;
pub mod daemon_cmd;
pub mod doctor_cmd;
pub mod index_cmd;
pub mod ocr_cmd;
pub mod query_cmd;

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
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
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
}

pub fn dispatch(cli: Cli, cfg: &AppConfig) -> Result<()> {
    match cli.command {
        Commands::Answer(args) => answer_cmd::run(args, cfg),
        Commands::Index(args) => index_cmd::run(args, cfg),
        Commands::Query(args) => query_cmd::run(args, cfg),
        Commands::Daemon(args) => daemon_cmd::run(args, cfg),
        Commands::Doctor => doctor_cmd::run(cfg),
        Commands::Ocr(args) => ocr_cmd::run(args, cfg),
        Commands::Calendar(args) => calendar_cmd::run(args, cfg),
    }
}
