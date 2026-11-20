# Indexing reliability and hybrid retrieval

Hydrogen now has a complete local retrieval path for lexical, document-semantic,
and cross-modal visual search. Model inference stays in the Rust process; no
Python worker or PyTorch runtime is required.

## Implemented

- BLAKE3 digests are loaded from SQLite in one read and checked during sync.
  Files whose bytes changed despite an unchanged size/mtime are reindexed.
- PDF page rows are replaced transactionally only after a successful extraction.
  Failed Poppler or OCR passes preserve the last known-good index and remain
  retryable.
- PDF previews are rendered on demand for the final visible search results.
  Scanned pages are rendered during indexing only when OCR needs their pixels.
- PDF extraction honours the configured page limit in Poppler itself, not just
  after collecting its output.
- PDF and image OCR jobs are atomically claimed, attempts are recorded, stale
  processing jobs are recovered after thirty minutes, and failures are retried.
- Image OCR distinguishes a successful empty recognition from inference failure.
  Failed OCR preserves previous OCR text and remains pending for retry.
- DOCX/ODT/EPUB/PPTX/XLSX XML extraction has per-member and aggregate
  decompressed-byte limits.
- Math-aware tokenization retains single-letter variables inside expressions
  while continuing to discard ordinary one-letter prose tokens.
- Optional quantized MiniLM document vectors provide semantic text retrieval.
- Optional CLIP ViT-B/32 text and image encoders share a vector space for
  text-to-image retrieval. Candidate lists are fused with lexical results using
  reciprocal-rank fusion.
- Optional SmolVLM-256M image captions are available behind the
  `image-captioning` build feature and an explicit runtime environment flag.
- `hydrogen doctor` reports dependencies, model/cache presence, SQLite
  integrity, queue states, vector counts, recent worker latency/RSS, and
  configured resource ceilings.
- End-to-end fixture tests exercise indexing, OCR retry, vector persistence,
  semantic retrieval, RRF, and lexical fallback when embedding models fail.

## Resource controls

- Inference is CPU-only and limited to at most two threads.
- Image processing uses configurable batches (default two).
- Image sync runs at low process priority and records batch duration/RSS.
- PDF indexing is bounded by `HYDROGEN_PDF_PAGE_LIMIT` (default 500, hard max 500).
- Document sync, PDF indexing, and image batches record worker metrics.
- Semantic models are lazy-loaded. One-shot `index embed` and `query` commands
  release model memory when the process exits. A long-running daemon retains
  models it has used, so enabling semantic indexing in a daemon trades memory
  for automatic vector updates.

## Deliberate boundaries

- Captioning is optional and off by default because its working set is larger
  than OCR or embeddings. CLIP vectors remain the primary visual retrieval signal.
- Existing PDF vectors can be backfilled with `hydrogen index embed`; enabling
  embeddings during later syncs automatically maintains vectors for changed files.
- The default CI checks the normal build. The optional captioning feature has a
  separate compile check so it cannot silently rot.
