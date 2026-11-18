#!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
VENV="${HYDROGEN_OCR_VENV:-$PREFIX/share/hydrogen-ocr-venv}"
PYTHON="${PYTHON:-python3}"

if ! command -v "$PYTHON" >/dev/null 2>&1; then
    printf 'Missing Python interpreter: %s\n' "$PYTHON" >&2
    exit 1
fi

"$PYTHON" -m venv "$VENV"
"$VENV/bin/python" -m pip install --upgrade pip wheel
# Install CPU-only PyTorch first. This avoids pulling a CUDA build onto the
# user's Intel-integrated-graphics laptop.
"$VENV/bin/python" -m pip install --index-url https://download.pytorch.org/whl/cpu torch torchvision
"$VENV/bin/python" -m pip install 'pix2text>=1.1.4' 'transformers>=4.38,<5' 'rapidocr_onnxruntime'

printf '\nHydrogen OCR dependencies installed.\n'
printf 'Pix2Text and TrOCR model weights are downloaded on first use.\n'
printf 'The first OCR run therefore needs an internet connection.\n'
printf 'Virtual environment: %s\n' "$VENV"
