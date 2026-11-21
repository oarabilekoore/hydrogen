//! On-device OCR engines. All inference stays in the Hydrogen process.
//!
//! PP-OCR is loaded lazily from models/ppocr/{det,rec}; Texo is loaded lazily
//! from models/texo-distill-f16.gguf. No Python, subprocess, or network access
//! is used at inference time.
use crate::domain::ocr::layout::OcrBox;
use crate::error::{HydrogenError, Result};
use crate::ports::extractor::{CaptionProvider, OcrProvider};
#[cfg(feature = "image-captioning")]
use ffai_argus::SmolVlm;
#[cfg(feature = "image-captioning")]
use ffai_core::engine::{VlmEngine, VlmOptions, VlmPrompt};
use pure_onnx_ocr::OcrEngine;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "image-captioning")]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock};

pub struct RustOcrProvider {
    models_dir: PathBuf,
    inference_threads: usize,
    printed: OnceLock<std::result::Result<Arc<OcrEngine>, String>>,
    formula: OnceLock<std::result::Result<Arc<texo_ocr::Ocr>, String>>,
    #[cfg(feature = "image-captioning")]
    captioner: OnceLock<std::result::Result<Mutex<SmolVlm>, String>>,
    #[cfg(feature = "image-captioning")]
    captions_enabled: bool,
}

impl RustOcrProvider {
    pub fn new(models_dir: PathBuf, inference_threads: usize, captions_enabled: bool) -> Self {
        Self {
            models_dir,
            inference_threads: inference_threads.clamp(1, 2),
            printed: OnceLock::new(),
            formula: OnceLock::new(),
            #[cfg(feature = "image-captioning")]
            captioner: OnceLock::new(),
            #[cfg(feature = "image-captioning")]
            captions_enabled,
        }
    }

    fn printed_engine(&self) -> Result<Arc<OcrEngine>> {
        let result = self.printed.get_or_init(|| {
            let det = self.models_dir.join("ppocr/det");
            let rec = self.models_dir.join("ppocr/rec");
            if !det.is_dir() || !rec.is_dir() {
                return Err(format!(
                    "PP-OCR models missing. Expected {} and {}",
                    det.display(),
                    rec.display()
                ));
            }
            pure_onnx_ocr::OcrEngineBuilder::new()
                .det_model_dir(det)
                .rec_model_dir(rec)
                .inference_threads(self.inference_threads)
                .rec_batch_size(1)
                .plan_cache_capacity(1, 2)
                .build()
                .map(Arc::new)
                .map_err(|e| e.to_string())
        });
        result.clone().map_err(HydrogenError::ExternalTool)
    }

    fn formula_engine(&self) -> Result<Arc<texo_ocr::Ocr>> {
        let result = self.formula.get_or_init(|| {
            let path = self.models_dir.join("texo-distill-f16.gguf");
            if !path.is_file() {
                return Err(format!("Texo formula model missing: {}", path.display()));
            }
            texo_ocr::Ocr::from_path(&path)
                .map(Arc::new)
                .map_err(|e| e.to_string())
        });
        result.clone().map_err(HydrogenError::ExternalTool)
    }

    /// Recognise a cropped mathematical expression and return LaTeX.
    pub fn recognize_formula(&self, image_path: &Path) -> Result<String> {
        self.formula_engine()?
            .recognize_file(image_path)
            .map_err(|e| HydrogenError::ExternalTool(e.to_string()))
    }

    fn run_printed(&self, path: &Path) -> Result<Vec<OcrBox>> {
        let engine = self.printed_engine()?;
        let results = engine
            .run_from_path(path)
            .map_err(|e| HydrogenError::ExternalTool(e.to_string()))?;
        Ok(results
            .into_iter()
            .filter_map(|item| {
                let points = item
                    .bounding_box
                    .exterior()
                    .0
                    .iter()
                    .map(|p| [p.x, p.y])
                    .collect::<Vec<_>>();
                if item.text.trim().is_empty() || points.is_empty() {
                    return None;
                }
                Some(OcrBox {
                    box_pts: points,
                    text: item.text,
                    score: f64::from(item.confidence),
                })
            })
            .collect())
    }
}

impl OcrProvider for RustOcrProvider {
    fn batch_ocr_text(&self, paths: &[String]) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for path in paths {
            let p = Path::new(path);
            match self.run_printed(p) {
                Ok(boxes) => {
                    // A successful empty result is distinct from inference failure.
                    let text = boxes
                        .into_iter()
                        .map(|b| b.text)
                        .collect::<Vec<_>>()
                        .join("\n");
                    out.insert(path.clone(), text);
                }
                Err(error) => {
                    eprintln!("hydrogen: printed OCR failed for {}: {error}", p.display());
                }
            }
        }
        out
    }

    fn ocr_boxes(&self, image_path: &Path) -> Result<Vec<OcrBox>> {
        self.run_printed(image_path)
    }

    fn ocr_math_latex(&self, image_path: &Path) -> Result<String> {
        self.recognize_formula(image_path)
    }
}

// SmolVLM is opt-in because even the 256M checkpoint has a much larger
// working set than OCR. It is loaded once per image-sync process, never at startup.
impl CaptionProvider for RustOcrProvider {
    fn batch_captions(&self, paths: &[String]) -> HashMap<String, String> {
        #[cfg(feature = "image-captioning")]
        let mut out = HashMap::new();
        #[cfg(not(feature = "image-captioning"))]
        let out = HashMap::new();
        #[cfg(feature = "image-captioning")]
        {
            if !self.captions_enabled || paths.is_empty() {
                return out;
            }
            let engine = self.captioner.get_or_init(|| {
                let manifest_dir = self.models_dir.join("argus");
                std::fs::create_dir_all(&manifest_dir).map_err(|e| e.to_string())?;
                let manifest_path = manifest_dir.join("smolvlm-256m-instruct.toml");
                if !manifest_path.is_file() {
                    std::fs::write(
                        &manifest_path,
                        include_str!("../../../models/smolvlm-256m-instruct.toml"),
                    )
                    .map_err(|e| e.to_string())?;
                }
                Ok(Mutex::new(SmolVlm::with_manifest_dir(manifest_dir)))
            });
            let Ok(engine) = engine else {
                eprintln!(
                    "hydrogen: SmolVLM initialisation failed: {}",
                    engine.as_ref().err().unwrap()
                );
                return out;
            };
            let Ok(mut engine) = engine.lock() else {
                eprintln!("hydrogen: SmolVLM model lock poisoned");
                return out;
            };
            for path in paths {
                let image = match ffai_media::load_image(Path::new(path)) {
                    Ok(image) => image,
                    Err(error) => {
                        eprintln!("hydrogen: image decode failed for {path}: {error}");
                        continue;
                    }
                };
                let instruction = "Describe this image in one concise factual sentence. Mention visible objects, document structure, diagrams, and readable text when relevant.";
                let prompt = VlmPrompt::single(&image, Some(instruction));
                let options = VlmOptions {
                    max_new_tokens: Some(48),
                    ..VlmOptions::default()
                };
                match engine.describe(&prompt, &options) {
                    Ok(caption) if !caption.trim().is_empty() => {
                        out.insert(path.clone(), caption.trim().to_string());
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("hydrogen: image caption failed for {path}: {error}"),
                }
            }
        }
        #[cfg(not(feature = "image-captioning"))]
        let _ = paths;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_paths_are_not_loaded_during_construction() {
        let provider = RustOcrProvider::new(PathBuf::from("/nonexistent/hydrogen-models"), 2, false);
        assert!(provider.printed.get().is_none());
        assert!(provider.formula.get().is_none());
    }
}
