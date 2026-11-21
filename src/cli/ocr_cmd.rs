use crate::adapters::ml::rust_ocr::RustOcrProvider;
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
    /// OCR a cliphist image and paste recognised text
    Clip {
        clip_id: String,
        #[arg(long, default_value = "")]
        window: String,
        /// Use the dedicated formula recognizer and return LaTeX
        #[arg(long)]
        math: bool,
    },
    /// OCR an image file and paste recognised text
    File {
        path: PathBuf,
        #[arg(long, default_value = "")]
        window: String,
        /// Use the dedicated formula recognizer and return LaTeX
        #[arg(long)]
        math: bool,
    },
}

pub fn run(args: OcrArgs, cfg: &AppConfig) -> Result<()> {
    let desktop = HyprlandDesktop;
    let ocr = RustOcrProvider::new(cfg.models_dir.clone(), cfg.inference_threads, cfg.settings.indexing.images.captions);
    let service = OcrService::new(&ocr, &desktop);
    match args.action {
        OcrAction::Clip {
            clip_id,
            window,
            math,
        } => {
            if math {
                service.ocr_clipboard_math(&clip_id, &window)
            } else {
                service.ocr_clipboard(&clip_id, &window)
            }
        }
        OcrAction::File { path, window, math } => {
            if math {
                service.ocr_file_math(&path, &window)
            } else {
                service.ocr_file(&path, &window)
            }
        }
    }
}
