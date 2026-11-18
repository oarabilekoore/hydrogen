# Local model layout

Set `HYDROGEN_MODELS_DIR` to override the default
`~/.local/share/hydrogen/models` directory.

Expected layout:

```text
models/
  ppocr/
    det/       # PaddleOCR ONNX detector export and inference.yml
    rec/       # PaddleOCR ONNX recognizer export, dictionary/config
  texo-distill-f16.gguf
  handwriting/ # reserved for the handwriting-specific recognizer
  embeddings/  # text and joint image/text encoders
```

Hydrogen does not download models during search or inference. Model acquisition
is an explicit setup step, allowing users to inspect model licences and avoid
surprise network access. PP-OCR model files retain their upstream Apache-2.0
licensing. Texo and its distributed weights are AGPL-3.0-only.

The current Rust OCR adapter implements PP-OCR printed-text detection and
Texo formula recognition. It does not claim that PP-OCR is a handwriting
model, that Texo recognises full pages, or that embeddings/captioning are
already implemented. Those are separate model adapters and index migrations.
