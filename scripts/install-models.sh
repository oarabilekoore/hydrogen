#!/usr/bin/env bash
set -euo pipefail

MODELS_DIR="${GSEARCH_MODELS_DIR:-$HOME/.local/share/gsearch/models}"
BASE="https://huggingface.co"
mkdir -p "$MODELS_DIR/ppocr/det" "$MODELS_DIR/ppocr/rec"

fetch() {
  local url="$1" output="$2"
  if [[ -s "$output" ]]; then
    printf 'Already present: %s\n' "$output"
    return
  fi
  printf 'Downloading %s\n' "$output"
  curl --fail --location --retry 3 --retry-delay 2 "$url" --output "$output"
}

# PP-OCRv6 Tiny is the smallest official PP-OCRv6 tier (~6 MB combined).
fetch "$BASE/PaddlePaddle/PP-OCRv6_tiny_det_onnx/resolve/main/inference.onnx" "$MODELS_DIR/ppocr/det/inference.onnx"
fetch "$BASE/PaddlePaddle/PP-OCRv6_tiny_det_onnx/resolve/main/inference.yml" "$MODELS_DIR/ppocr/det/inference.yml"
fetch "$BASE/PaddlePaddle/PP-OCRv6_tiny_rec_onnx/resolve/main/inference.onnx" "$MODELS_DIR/ppocr/rec/inference.onnx"
fetch "$BASE/PaddlePaddle/PP-OCRv6_tiny_rec_onnx/resolve/main/inference.yml" "$MODELS_DIR/ppocr/rec/inference.yml"

# Texo is AGPL-3.0-only. The user must accept the model repository's licence.
fetch "$BASE/cstr/texo-distill-gguf/resolve/main/texo-distill-f16.gguf" "$MODELS_DIR/texo-distill-f16.gguf"

printf '\nModels installed under %s\n' "$MODELS_DIR"
