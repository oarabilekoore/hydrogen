# Local model setup

Set `HYDROGEN_MODELS_DIR` to override the default
`~/.local/share/hydrogen/models` directory.

Install the current Rust OCR models with:

```sh
make models
```

This downloads:
- PP-OCRv6 Tiny detector and recognizer ONNX exports plus their configuration.
  The official tiny tier is approximately 6 MB combined and is Apache-2.0.
- Texo Distill FP16 GGUF for mathematical formula recognition. The model is
  approximately 39 MB and is AGPL-3.0-only.

Expected layout:

```text
models/
  ppocr/
    det/inference.onnx
    det/inference.yml
    rec/inference.onnx
    rec/inference.yml
  texo-distill-f16.gguf
  handwriting/       # future dedicated handwritten prose model
  embeddings/        # future text and joint image/text encoders
```

The Rust PP-OCR adapter loads both stages from their directories. Texo loads
only when a formula OCR request is made. The model files are not bundled into
the Hydrogen binary and no network access is performed during inference.

## Current support boundary

- Printed text: PP-OCRv6 Tiny through the pure-Rust `pure-onnx-ocr` pipeline.
- Formula crops: Texo Rust recognizer, output as LaTeX.
- Handwritten formula recognition: Texo's published evaluation includes
  handwritten mathematical expressions. This does not make it a general
  handwritten-prose recognizer.
- Handwritten prose: a dedicated TrOCR handwritten-line adapter is not wired
  yet. The PP-OCR path may read neat handwriting, but it is not advertised as
  handwriting-specialised.
- PDF page OCR uses one-pass text extraction and renders only scanned pages that require OCR.
- Text semantic retrieval uses MiniLM; shared CLIP vectors support image-to-text and image-to-image retrieval.
- Hybrid search fuses lexical, document-vector, and visual-vector candidates with reciprocal-rank fusion.

Model weights retain their upstream licences. PP-OCR is Apache-2.0; Texo is
AGPL-3.0-only. Hydrogen itself is AGPL-3.0-only.

## Optional semantic embeddings and image captions

Semantic indexing is opt-in so a normal Hydrogen sync does not download models or
spend CPU time unexpectedly.

Set `HYDROGEN_EMBEDDINGS=1` before `hydrogen index sync` to enable:
- Quantized all-MiniLM-L6-v2 for document text vectors (384 dimensions).
- CLIP ViT-B/32 text and vision encoders for a shared image/text space (512 dimensions).

Both run locally through FastEmbed/ONNX Runtime with a maximum of two inference
threads. Models are cached below `$HYDROGEN_MODELS_DIR/embeddings/fastembed`.
Run `hydrogen index embed` once to backfill an existing index. Use `HYDROGEN_EMBEDDINGS=1`
for automatic updates during future syncs. Use `hydrogen query "..." --semantic` to combine lexical, document-vector and
cross-modal image retrieval with reciprocal-rank fusion. Use `hydrogen query --image ./query.png` for image-to-image retrieval. First use downloads the
selected model files; subsequent inference is offline.

For free-form image descriptions, build with the optional Rust-native Candle SmolVLM
backend and enable it explicitly:

```sh
cargo build --release --features image-captioning
HYDROGEN_CAPTIONS=1 hydrogen index sync-images
```

The manifest is included in Hydrogen and copied into
`$HYDROGEN_MODELS_DIR/argus` on first use. Fetch the checkpoint into the same
local model tree before enabling captions:

```sh
cargo install ffai-cli
FFAI_CACHE="$HYDROGEN_MODELS_DIR/argus/cache" ffai models --fetch smolvlm-256m-instruct
```

Captioning is disabled by default because its working memory is substantially
larger than OCR or embedding inference. The model is loaded only inside the
image-sync process. CLIP vectors remain the primary visual retrieval signal;
generated captions are optional searchable metadata, not a replacement for
embeddings. `hydrogen doctor` reports model availability, SQLite integrity,
queue state, embedding counts, recent worker latency/RSS, and active resource
ceilings.
