# Hydrogen multimodal architecture

Hydrogen is a local-first Rust application. Python is not part of the runtime or
model execution path. Models are files stored below `$HYDROGEN_MODELS_DIR`
(default: `~/.local/share/hydrogen/models`) and are loaded lazily.

## Runtime budget

The default inference budget is two CPU threads and an image batch size of two
(configurable within 1-4). PDF indexing is capped at 500 pages by default. Model
initialisation is lazy; a normal lexical sync does not download embedding models.
The image-sync process is separate from the launcher, records elapsed time and
process RSS per batch, and exits after the job so its model memory is reclaimed.
Optional SmolVLM captioning is disabled by default because it has a much larger
working set. These defaults target an 8 GB system, not a workstation GPU.

## Modalities

- Printed text: PP-OCR DB detector + CTC recognizer through pure Rust
  `pure-onnx-ocr` / tract. Place the PP-OCR detector export in
  `models/ppocr/det` and recognizer export in `models/ppocr/rec`.
- Equations: Texo OCR, a Rust-native formula recognizer. It accepts formula
  crops and emits LaTeX. It is not a page-layout OCR engine; page layout and
  formula-region detection remain separate responsibilities.
- Handwriting: route notebook-like pages through handwriting-specific
  recognition rather than treating printed OCR as equivalent. The provider
  interface must preserve region coordinates and confidence so a dedicated
  handwriting model can be added without changing indexing or search.
- PDFs: extract embedded text in one Poppler pass. Render only image-only pages
  that require OCR and pages in the final visible search results. Store page number,
  page text and any generated preview path separately from the parent file.
- Images: retain OCR and visual embeddings as separate channels. Captions are
  optional metadata, never a substitute for image embeddings.

## Retrieval

Search combines filename/path matching, SQLite FTS5/BM25, math-aware tokens,
MiniLM document vectors, and CLIP image vectors. Text queries fuse lexical,
document-vector and CLIP image results with reciprocal-rank fusion. CLIP's
matching text and vision encoders share a 512-dimensional vector space, so the
CLI supports text-to-image and image-to-image queries. Document vectors use a
separate quantized 384-dimensional MiniLM space.

## Index integrity

Use BLAKE3 content hashes rather than relying only on size and mtime. Inotify events trigger
immediate incremental jobs; periodic root reconciliation repairs dropped events.
Deletes remove all page, embedding, FTS and cache rows in the same transaction.
New files become searchable lexically before expensive OCR/embedding finishes.

## Compatibility and migration

Existing document rows remain valid. Page rows and vectors are additive schema
migrations. Rebuild vector data in the background and retain lexical search while
models are unavailable. A model missing from disk must produce a clear status,
not a silent empty OCR result.
