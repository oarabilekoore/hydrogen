# Machine-learning adapters

This package wraps inference runtimes and model-specific preprocessing. It should expose stable Hydrogen-level operations while keeping runtime and model details private.

## Rust references

- `src/adapters/ml/embeddings.rs`
- `src/adapters/ml/rust_ocr.rs`
- Inspect the current model manager, image-captioning, and formula OCR code as well.

## Existing model behaviour to preserve

The Rust implementation has used MiniLM embeddings (384 dimensions), CLIP text/image embeddings (512 dimensions), PP-OCR text detection/recognition, formula OCR, and optional image captioning. Verify the exact model IDs, versions, quantization, preprocessing, normalization, and output dimensions from the current source and model metadata before implementing.

## Candidate approach

ONNX Runtime's C API may allow Odin to run ONNX models without recreating an inference engine. It does not automatically replace tokenizers, image preprocessing, postprocessing, model downloads, or model-specific decoding. Prototype one model end-to-end before committing to this architecture.

Formula OCR is a separate risk: a GGUF weight file is not an inference engine, and a Rust-only crate cannot be called directly from Odin without a stable C ABI wrapper or a replacement.

## Suggested learning order

1. Model metadata and checksums.
2. Runtime initialization and session lifecycle.
3. Text tokenization and padding.
4. Tensor construction and inference.
5. Output decoding and normalization.
6. Image preprocessing and CLIP.
7. OCR detection, crop generation, recognition, and reading order.
8. Model cache, cancellation, errors, and resource cleanup.

Resources: [ONNX Runtime C API](https://onnxruntime.ai/docs/api/c/), [ONNX Runtime concepts](https://onnxruntime.ai/docs/), [Odin foreign system](https://odin-lang.org/docs/overview/#foreign-system).
