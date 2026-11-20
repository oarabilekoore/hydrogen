# Hydrogen multimodal architecture

Hydrogen is a local-first Rust application. Python and PyTorch are not part of
the runtime or model execution path. Models are stored below
`$HYDROGEN_MODELS_DIR` (default: `~/.local/share/hydrogen/models`) and loaded
lazily when the corresponding operation is requested.

## Runtime budget

The default CPU inference budget is two threads and one small image batch at a
time. Image indexing runs at low process priority. PDF extraction is bounded by
the configured page limit, and previews are rendered only for scanned-page OCR
or final visible PDF search results. Worker duration and resident memory are
recorded in SQLite for `hydrogen doctor`.

Model objects remain resident until their process exits. One-shot commands such
as `hydrogen index embed` and `hydrogen query --semantic` naturally release
them on exit. A long-running daemon retains models it has loaded; keep
`HYDROGEN_EMBEDDINGS` disabled in the daemon when memory is more important than
automatic semantic-vector refresh.

## Modalities

- Printed text: PP-OCR DB detector + CTC recognizer through pure Rust
  `pure-onnx-ocr` / tract.
- Equations: Texo OCR, a Rust-native formula recognizer. It accepts formula
  crops and emits LaTeX. It is not a page-layout OCR engine.
- Handwriting: Texo supports mathematical expressions, but Hydrogen does not
  claim a dedicated handwritten-prose recognizer.
- PDFs: extract embedded text first. Image-only pages are rendered and sent
  through OCR. Store page number, page text, page hash and preview metadata
  separately from the parent file.
- Images: retain OCR, captions, and visual embeddings as separate channels.
  Captions are optional metadata, never a substitute for image embeddings.
- Text vectors: quantized MiniLM embeddings for document passages.
- Cross-modal vectors: CLIP ViT-B/32 text and image encoders share a space,
  allowing text queries to retrieve visually relevant images.

## Retrieval

Search combines filename/path lexical matching, SQLite FTS5/BM25, sparse
TF-IDF/math tokens, MiniLM document vectors, and CLIP image vectors. Ranked
lists are fused with reciprocal-rank fusion so a strong result from one channel
is not discarded by another. Semantic channels are optional: if a model is
missing or inference fails, lexical search still returns results.

## Index integrity

Use BLAKE3 content hashes rather than relying only on size and mtime. Inotify
events trigger incremental jobs; periodic root reconciliation repairs dropped
events. Deletes remove page, embedding, FTS and cache rows transactionally. New
files become searchable lexically before expensive OCR/embedding finishes.

## Compatibility and migration

Existing document rows remain valid. Page rows and vectors are additive schema
migrations. Rebuild vector data with `hydrogen index embed` while lexical search
remains available. Missing model files produce diagnostics rather than silently
turning semantic search into an empty result.
