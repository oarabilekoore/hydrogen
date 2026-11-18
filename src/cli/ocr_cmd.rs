use crate::adapters::ml::workers::RapidOcrWorker;
use crate::adapters::wayland::hyprland::HyprlandDesktop;
use crate::application::ocr_service::OcrService;
use crate::config::AppConfig;
use crate::error::Result;
use clap::{Args, Subcommand, ValueEnum};
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
        /// Recognition engine: auto/math for equations, handwriting for handwritten notes, rapid for legacy OCR
        #[arg(long, value_enum, default_value_t = OcrEngine::Auto)]
        engine: OcrEngine,
    },
    /// Run OCR on an image file, copy Markdown, and optionally paste
    File {
        /// Path to image file
        path: PathBuf,
        /// Optional Hyprland window address to focus and paste into
        #[arg(long, default_value = "")]
        window: String,
        /// Recognition engine: auto/math for equations, handwriting for handwritten notes, rapid for legacy OCR
        #[arg(long, value_enum, default_value_t = OcrEngine::Auto)]
        engine: OcrEngine,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OcrEngine {
    /// Mixed text and mathematical formula recognition with LaTeX output
    Auto,
    /// Mixed text and mathematical formula recognition with LaTeX output
    Math,
    /// Handwritten English line recognition
    Handwriting,
    /// Original RapidOCR detector/recognizer
    Rapid,
}

impl OcrEngine {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Math => "math",
            Self::Handwriting => "handwriting",
            Self::Rapid => "rapid",
        }
    }
}

pub fn run(args: OcrArgs, cfg: &AppConfig) -> Result<()> {
    let desktop = HyprlandDesktop;

    match args.action {
        OcrAction::Clip { clip_id, window, engine } => {
            let ocr = RapidOcrWorker::new_document(
                cfg.rapidocr_python.clone(),
                cfg.ocr_python.clone(),
                cfg.ocr_worker_script.clone(),
                engine.as_str().into(),
            );
            OcrService::new(&ocr, &desktop).ocr_clipboard(&clip_id, &window)
        }
        OcrAction::File { path, window, engine } => {
            let ocr = RapidOcrWorker::new_document(
                cfg.rapidocr_python.clone(),
                cfg.ocr_python.clone(),
                cfg.ocr_worker_script.clone(),
                engine.as_str().into(),
            );
            OcrService::new(&ocr, &desktop).ocr_file(&path, &window)
        }
    }
}
