use crate::adapters::ml::workers::RapidOcrWorker;
use crate::adapters::wayland::hyprland::HyprlandDesktop;
use crate::application::ocr_service::OcrService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct OcrArgs {
    #[command(subcommand)]
    pub action: OcrAction,
}

#[derive(Subcommand, Debug)]
pub enum OcrAction {
    /// Decode a cliphist entry ID, run OCR, copy Markdown, and optionally paste
    Clip {
        /// cliphist entry ID
        clip_id: String,
        /// Optional Hyprland window address to focus and paste into
        #[arg(long, default_value = "")]
        window: String,
    },
    /// Run OCR on an image file, copy Markdown, and optionally paste
    File {
        /// Path to image file
        path: PathBuf,
        /// Optional Hyprland window address to focus and paste into
        #[arg(long, default_value = "")]
        window: String,
    },
}

pub fn run(args: OcrArgs, cfg: &AppConfig) -> Result<()> {
    let ocr = RapidOcrWorker::new(cfg.rapidocr_python.clone());
    let desktop = HyprlandDesktop;
    let service = OcrService::new(&ocr, &desktop);

    match args.action {
        OcrAction::Clip { clip_id, window } => service.ocr_clipboard(&clip_id, &window),
        OcrAction::File { path, window } => service.ocr_file(&path, &window),
    }
}
