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
- PDF page OCR, image embeddings, text embeddings, and image-to-image retrieval
  are represented in the architecture/schema but are not yet wired end-to-end.

Model weights retain their upstream licences. PP-OCR is Apache-2.0; Texo is
AGPL-3.0-only. Hydrogen itself is AGPL-3.0-only.
