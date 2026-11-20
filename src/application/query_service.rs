use crate::adapters::ml::embeddings::{CLIP_MODEL_ID, TEXT_MODEL_ID};
use crate::domain::document::SearchHit;
use crate::error::Result;
use crate::ports::extractor::EmbeddingProvider;
use crate::ports::repository::IndexRepository;
use std::collections::HashMap;

pub struct QueryService<'a> {
    repo: &'a dyn IndexRepository,
    embeddings: &'a dyn EmbeddingProvider,
}

impl<'a> QueryService<'a> {
    pub fn new(repo: &'a dyn IndexRepository, embeddings: &'a dyn EmbeddingProvider) -> Self {
        Self { repo, embeddings }
    }

    pub fn search_image(&self, image_path: &std::path::Path, limit: usize) -> Result<Vec<SearchHit>> {
        let vector = self.embeddings.embed_image(image_path)?;
        self.repo
            .semantic_search(&vector, CLIP_MODEL_ID, "clip", limit.max(1))
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
        mode: &str,
        semantic: bool,
    ) -> Result<Vec<SearchHit>> {
        let trimmed = query.trim();
        if trimmed.chars().count() < 2 {
            return Ok(Vec::new());
        }
        if !semantic || mode == "files" {
            return self.repo.search(trimmed, limit.max(1), mode);
        }

        // Keep the lexical engine as a first-class retrieval channel. Semantic
        // vectors augment it; they never replace exact filename/phrase matches.
        let lexical = self
            .repo
            .search(trimmed, limit.saturating_mul(3).max(30), mode)?;
        let mut channels = vec![lexical];

        // Semantic models are optional and downloaded on first use. A missing
        // model or transient inference failure must never disable lexical search.
        match self.embeddings.embed_document(trimmed) {
            Ok(vector) => match self.repo.semantic_search(
                &vector,
                TEXT_MODEL_ID,
                "text",
                limit.saturating_mul(5).max(50),
            ) {
                Ok(hits) => channels.push(hits),
                Err(error) => eprintln!("hydrogen: text-vector search unavailable: {error}"),
            },
            Err(error) => eprintln!("hydrogen: text embeddings unavailable: {error}"),
        }

        match self.embeddings.embed_clip_text(trimmed) {
            Ok(vector) => match self.repo.semantic_search(
                &vector,
                CLIP_MODEL_ID,
                "clip",
                limit.saturating_mul(5).max(50),
            ) {
                Ok(hits) => channels.push(hits),
                Err(error) => eprintln!("hydrogen: cross-modal search unavailable: {error}"),
            },
            Err(error) => eprintln!("hydrogen: CLIP text encoder unavailable: {error}"),
        }

        Ok(reciprocal_rank_fusion(channels, limit))
    }
}

fn reciprocal_rank_fusion(channels: Vec<Vec<SearchHit>>, limit: usize) -> Vec<SearchHit> {
    const K: f64 = 60.0;
    let mut fused: HashMap<String, (SearchHit, f64)> = HashMap::new();
    for channel in channels {
        for (rank, hit) in channel.into_iter().enumerate() {
            let contribution = 1.0 / (K + rank as f64 + 1.0);
            let entry = fused
                .entry(hit.path.clone())
                .or_insert_with(|| (hit.clone(), 0.0));
            entry.1 += contribution;
            if entry.0.snippet.is_empty() && !hit.snippet.is_empty() {
                entry.0.snippet = hit.snippet;
            }
            entry.0.vec = entry.0.vec.max(hit.vec);
            entry.0.phrase |= hit.phrase;
        }
    }
    let mut results: Vec<SearchHit> = fused
        .into_values()
        .map(|(mut hit, score)| {
            hit.score = score * 10_000.0;
            hit
        })
        .collect();
    results.sort_by(|a, b| b.score.total_cmp(&a.score));
    results.truncate(limit.max(1));
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str) -> SearchHit {
        SearchHit {
            path: path.into(),
            name: path.into(),
            source: "doc".into(),
            score: 0.0,
            vec: 0.0,
            fts: 0.0,
            coverage: 0.0,
            phrase: false,
            snippet: String::new(),
            is_dir: false,
            page: None,
            line: None,
            preview: String::new(),
        }
    }

    #[test]
    fn rrf_rewards_documents_found_by_multiple_channels() {
        let fused = reciprocal_rank_fusion(
            vec![
                vec![hit("shared"), hit("lexical-only")],
                vec![hit("shared"), hit("semantic-only")],
            ],
            10,
        );
        assert_eq!(fused[0].path, "shared");
    }

    #[test]
    fn rrf_preserves_unique_candidates() {
        let fused = reciprocal_rank_fusion(vec![vec![hit("one")], vec![hit("two")]], 10);
        assert_eq!(fused.len(), 2);
    }
}
