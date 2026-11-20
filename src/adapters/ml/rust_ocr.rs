//! On-device OCR engines. All inference stays in the Hydrogen process.
//!
//! PP-OCR is loaded lazily from models/ppocr/{det,rec}; Texo is loaded lazily
//! from models/texo-distill-f16.gguf. No Python, subprocess, or network access
//! is used at inference time.
use crate::domain::ocr::layout::OcrBox;
use crate::error::{HydrogenError, Result};
use crate::ports::extractor::{CaptionProvider, OcrProvider};
use pure_onnx_ocr::OcrEngine;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

pub struct RustOcrProvider {
    models_dir: PathBuf,
    printed: OnceLock<std::result::Result<Arc<OcrEngine>, String>>,
    formula: OnceLock<std::result::Result<Arc<texo_ocr::Ocr>, String>>,
}

impl RustOcrProvider {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models_dir,
            printed: OnceLock::new(),
            formula: OnceLock::new(),
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
                .inference_threads(2)
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
                    let text = boxes.into_iter().map(|b| b.text).collect::<Vec<_>>().join("\n");
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

// Caption generation is deliberately not faked with OCR text. A real joint
// image/text encoder is provided by the embedding subsystem in the next layer.
impl CaptionProvider for RustOcrProvider {
    fn batch_captions(&self, _paths: &[String]) -> HashMap<String, String> {
        HashMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_paths_are_not_loaded_during_construction() {
        let provider = RustOcrProvider::new(PathBuf::from("/nonexistent/hydrogen-models"));
        assert!(provider.printed.get().is_none());
        assert!(provider.formula.get().is_none());
    }
}
