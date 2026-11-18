# Hydrogen multimodal architecture

Hydrogen is a local-first Rust application. Python is not part of the runtime or
model execution path. Models are files stored below `$HYDROGEN_MODELS_DIR`
(default: `~/.local/share/hydrogen/models`) and are loaded lazily.

## Runtime budget

The default CPU inference budget is two threads and one image at a time. The
indexer must never run two large model families concurrently. OCR and embedding
workers are background jobs, yield to interactive queries, and persist progress.
The model manager owns loading/unloading and must not keep unused model weights
resident. These defaults target an 8 GB system, not a workstation GPU.

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
- PDFs: extract embedded text first. If the page is image-only or text density
  is too low, render the page and route it through OCR. Store page number,
  page text, page hash and preview metadata separately from the parent file.
- Images: retain OCR and visual embeddings as separate channels. Captions are
  optional metadata, never a substitute for image embeddings.

## Retrieval

Search should combine filename/path lexical matching, SQLite FTS5/BM25,
text embeddings, formula tokens/LaTeX, and image embeddings. Fuse ranked lists
with reciprocal-rank fusion so one channel cannot erase a useful result from
another. Query intent adjusts channel weights, not channel availability.
Text-to-image and image-to-image both use the same joint image/text embedding
space; image similarity also supports image-only queries.

## Index integrity

Use BLAKE3 content hashes rather than relying only on size and mtime. Page hashes
are derived from rendered page pixels plus extracted text. Inotify events trigger
immediate incremental jobs; periodic root reconciliation repairs dropped events.
Deletes remove all page, embedding, FTS and cache rows in the same transaction.
New files become searchable lexically before expensive OCR/embedding finishes.

## Compatibility and migration

Existing document rows remain valid. Page rows and vectors are additive schema
migrations. Rebuild vector data in the background and retain lexical search while
models are unavailable. A model missing from disk must produce a clear status,
not a silent empty OCR result.
