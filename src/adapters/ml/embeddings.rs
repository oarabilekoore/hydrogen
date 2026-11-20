//! Lazy, CPU-bounded local embedding models.
//!
//! Document retrieval uses quantized MiniLM. Cross-modal image retrieval uses
//! the matching CLIP ViT-B/32 text and vision towers, which share one vector
//! space. No model is loaded until an embedding operation is requested.
use crate::error::{HydrogenError, Result};
use crate::ports::extractor::EmbeddingProvider;
use fastembed::{
    EmbeddingModel, ImageEmbedding, ImageEmbeddingModel, ImageInitOptions,
    TextEmbedding, TextInitOptions,
};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const TEXT_MODEL_ID: &str = "all-minilm-l6-v2-q";
pub const CLIP_MODEL_ID: &str = "clip-vit-b32";
const DOCUMENT_CHUNK_CHARS: usize = 1_200;
const MAX_DOCUMENT_CHUNKS: usize = 8;

pub struct FastEmbedProvider {
    cache_dir: PathBuf,
    threads: usize,
    text: OnceLock<std::result::Result<Mutex<TextEmbedding>, String>>,
    clip_text: OnceLock<std::result::Result<Mutex<TextEmbedding>, String>>,
    clip_image: OnceLock<std::result::Result<Mutex<ImageEmbedding>, String>>,
}

impl FastEmbedProvider {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            cache_dir: models_dir.join("embeddings").join("fastembed"),
            threads: std::env::var("HYDROGEN_INFERENCE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(2).clamp(1, 2),
            text: OnceLock::new(),
            clip_text: OnceLock::new(),
            clip_image: OnceLock::new(),
        }
    }

    fn text_model(&self) -> Result<&Mutex<TextEmbedding>> {
        self.text
            .get_or_init(|| {
                TextEmbedding::try_new(
                    TextInitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
                        .with_cache_dir(self.cache_dir.join("text-minilm"))
                        .with_intra_threads(self.threads)
                        .with_show_download_progress(true),
                )
                .map(Mutex::new)
                .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(|e| HydrogenError::ExternalTool(format!("MiniLM embedding model: {e}")))
    }

    fn clip_text_model(&self) -> Result<&Mutex<TextEmbedding>> {
        self.clip_text
            .get_or_init(|| {
                TextEmbedding::try_new(
                    TextInitOptions::new(EmbeddingModel::ClipVitB32)
                        .with_cache_dir(self.cache_dir.join("clip-vit-b32-text"))
                        .with_intra_threads(self.threads)
                        .with_show_download_progress(true),
                )
                .map(Mutex::new)
                .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(|e| HydrogenError::ExternalTool(format!("CLIP text model: {e}")))
    }

    fn clip_image_model(&self) -> Result<&Mutex<ImageEmbedding>> {
        self.clip_image
            .get_or_init(|| {
                ImageEmbedding::try_new(
                    ImageInitOptions::new(ImageEmbeddingModel::ClipVitB32)
                        .with_cache_dir(self.cache_dir.join("clip-vit-b32-image"))
                        .with_intra_threads(self.threads)
                        .with_show_download_progress(true),
                )
                .map(Mutex::new)
                .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(|e| HydrogenError::ExternalTool(format!("CLIP vision model: {e}")))
    }
}

fn normalise_mean(vectors: Vec<Vec<f32>>) -> Result<Vec<f32>> {
    let first = vectors.first().ok_or_else(|| {
        HydrogenError::ExternalTool("embedding model returned no vectors".into())
    })?;
    let dimensions = first.len();
    if dimensions == 0 || vectors.iter().any(|v| v.len() != dimensions) {
        return Err(HydrogenError::ExternalTool(
            "embedding model returned inconsistent vector dimensions".into(),
        ));
    }
    let mut mean = vec![0.0f32; dimensions];
    for vector in &vectors {
        for (dst, value) in mean.iter_mut().zip(vector) {
            *dst += *value;
        }
    }
    let count = vectors.len() as f32;
    for value in &mut mean {
        *value /= count;
    }
    let norm = mean.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in &mut mean {
            *value /= norm;
        }
    }
    Ok(mean)
}

fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.chars().count() + word.chars().count() + 1 > DOCUMENT_CHUNK_CHARS
            && !current.is_empty()
        {
            out.push(std::mem::take(&mut current));
            if out.len() >= MAX_DOCUMENT_CHUNKS {
                break;
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() && out.len() < MAX_DOCUMENT_CHUNKS {
        out.push(current);
    }
    if out.is_empty() {
        out.push(text.chars().take(DOCUMENT_CHUNK_CHARS).collect());
    }
    out
}

impl EmbeddingProvider for FastEmbedProvider {
    fn embed_document(&self, text: &str) -> Result<Vec<f32>> {
        let text = text.trim();
        if text.is_empty() {
            return Err(HydrogenError::InvalidInput(
                "cannot embed an empty document".into(),
            ));
        }
        let inputs = chunks(text);
        let mut model = self
            .text_model()?
            .lock()
            .map_err(|_| HydrogenError::ExternalTool("MiniLM model lock poisoned".into()))?;
        let vectors = model
            .embed(inputs, Some(4))
            .map_err(|e| HydrogenError::ExternalTool(e.to_string()))?;
        normalise_mean(vectors)
    }

    fn embed_clip_text(&self, text: &str) -> Result<Vec<f32>> {
        let text = text.trim();
        if text.is_empty() {
            return Err(HydrogenError::InvalidInput(
                "cannot embed an empty CLIP query".into(),
            ));
        }
        let input = text.chars().take(400).collect::<String>();
        let mut model = self
            .clip_text_model()?
            .lock()
            .map_err(|_| HydrogenError::ExternalTool("CLIP text model lock poisoned".into()))?;
        let vectors = model
            .embed(vec![input], Some(1))
            .map_err(|e| HydrogenError::ExternalTool(e.to_string()))?;
        normalise_mean(vectors)
    }

    fn embed_image(&self, path: &Path) -> Result<Vec<f32>> {
        if !path.is_file() {
            return Err(HydrogenError::InvalidInput(format!(
                "image does not exist: {}",
                path.display()
            )));
        }
        let mut model = self
            .clip_image_model()?
            .lock()
            .map_err(|_| HydrogenError::ExternalTool("CLIP vision model lock poisoned".into()))?;
        let vectors = model
            .embed(vec![path.to_path_buf()], Some(1))
            .map_err(|e| HydrogenError::ExternalTool(e.to_string()))?;
        normalise_mean(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_is_lazy() {
        let provider = FastEmbedProvider::new(PathBuf::from("/tmp/hydrogen-models"));
        assert!(provider.text.get().is_none());
        assert!(provider.clip_text.get().is_none());
        assert!(provider.clip_image.get().is_none());
    }

    #[test]
    fn document_chunks_are_bounded() {
        let text = "engineering ".repeat(4_000);
        let parts = chunks(&text);
        assert!(parts.len() <= MAX_DOCUMENT_CHUNKS);
        assert!(parts.iter().all(|part| part.chars().count() <= DOCUMENT_CHUNK_CHARS));
    }

    #[test]
    fn mean_vectors_are_normalised() {
        let vector = normalise_mean(vec![vec![3.0, 0.0], vec![0.0, 4.0]]).unwrap();
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }
}
