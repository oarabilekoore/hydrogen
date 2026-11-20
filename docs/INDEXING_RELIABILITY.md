# Indexing reliability and search quality

This change set hardens the existing local-first indexing pipeline without adding
another inference runtime or changing the installed OCR models.

## Implemented

- BLAKE3 digests are loaded from SQLite in one read and checked during sync.
  Files whose bytes changed despite an unchanged size/mtime are reindexed.
- PDF page rows are replaced transactionally only after a non-empty extraction.
  A failed Poppler pass or failed page OCR leaves the last known-good page and
  document index intact. PDF digests are committed only after a successful pass,
  so a later sync retries transient failures.
- Image OCR distinguishes a successful empty recognition from a model failure.
  Failed inference is logged and is not written to the image cache as a successful
  empty result.
- DOCX/ODT/EPUB/PPTX/XLSX XML extraction has per-member and aggregate
  decompressed-byte limits.
- Math-aware tokenization retains single-letter variables in expressions while
  continuing to discard ordinary one-letter prose tokens. This improves matching
  for expressions such as `F = ma` and `x^2 + y^2 = r^2`.

## Deliberately not added in this change

The current repository does not yet contain a complete consumer for the
`model_jobs` table, a working image-caption model, or an end-to-end local
embedding provider. Adding placeholder workers or claiming that the existing
embedding table provides semantic retrieval would make the system less reliable,
not more. These need separate, testable implementations:

1. A durable worker that claims jobs atomically, records attempts, retries
   recoverable failures, and marks terminal outcomes.
2. A model-availability and queue diagnostic command.
3. A real local text embedding provider and a separately evaluated joint
   image/text model, followed by reciprocal-rank fusion with BM25 and math search.
4. On-demand PDF preview rendering and bounded worker concurrency.
5. End-to-end fixture tests for scanned PDFs, image OCR failures, interrupted
   extraction, same-size/same-mtime edits, and mathematical queries.

## Resource constraints

No new model weights or Python/PyTorch dependency is introduced. Hashing streams
files through a fixed-size buffer. Archive extraction is bounded. The existing
PP-OCRv6 Tiny and Texo model choices remain unchanged.
