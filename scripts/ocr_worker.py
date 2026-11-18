#!/usr/bin/env python3
"""On-demand CPU OCR worker for Hydrogen.

Engines:
  auto/math: Pix2Text mixed text + mathematical formula recognition (LaTeX).
  handwriting: TrOCR small handwritten, applied to line crops.
  rapid: legacy RapidOCR text detection/recognition.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


def emit_markdown(text: str, image_path: Path) -> None:
    from PIL import Image

    with Image.open(image_path) as image:
        width, height = image.size
    text = (text or "").strip()
    boxes = []
    if text:
        boxes.append({
            "box_pts": [[0.0, 0.0], [float(width), 0.0],
                        [float(width), float(height)], [0.0, float(height)]],
            "text": text,
            "score": 1.0,
        })
    print(json.dumps(boxes, ensure_ascii=False))


def run_math(image_path: Path) -> str:
    # Import the text/formula recognizer directly. Constructing Pix2Text's full
    # page pipeline also loads a layout detector that is unnecessary for a
    # single mixed text/formula crop and wastes RAM on this machine.
    import torch
    from pix2text.text_formula_ocr import TextFormulaOCR

    torch.set_num_threads(max(1, min(4, os.cpu_count() or 1)))
    torch.set_num_interop_threads(1)
    recognizer = TextFormulaOCR.from_config(
        None, enable_formula=True, device="cpu"
    )
    result = recognizer.recognize(
        str(image_path),
        return_text=True,
        resized_shape=768,
        mfr_batch_size=1,
        auto_line_break=True,
        embed_sep=(" $", "$ "),
        isolated_sep=("$$\n", "\n$$"),
    )
    return result if isinstance(result, str) else "\n".join(map(str, result))


def _line_regions(image):
    """Split a notebook page into horizontal text-line crops by projection."""
    import numpy as np

    gray = np.asarray(image.convert("L"))
    # A simple global threshold is predictable on clean scans and screenshots.
    ink = gray < 210
    row_ink = ink.sum(axis=1)
    active = row_ink >= max(2, int(gray.shape[1] * 0.001))
    indices = np.flatnonzero(active)
    if indices.size == 0:
        return []

    groups = []
    start = previous = int(indices[0])
    for value in indices[1:]:
        value = int(value)
        if value - previous > 12:
            if previous - start >= 4:
                groups.append((start, previous + 1))
            start = value
        previous = value
    if previous - start >= 4:
        groups.append((start, previous + 1))

    # Merge unusually close bands; this keeps dots/accents attached to letters.
    merged = []
    for top, bottom in groups:
        if merged and top - merged[-1][1] <= 5:
            merged[-1] = (merged[-1][0], bottom)
        else:
            merged.append((top, bottom))

    width, height = image.size
    regions = []
    for top, bottom in merged[:100]:
        top = max(0, top - 10)
        bottom = min(height, bottom + 10)
        crop_ink = ink[top:bottom, :]
        columns = np.flatnonzero(crop_ink.sum(axis=0) > 0)
        if columns.size == 0:
            continue
        left = max(0, int(columns[0]) - 14)
        right = min(width, int(columns[-1]) + 15)
        if right - left < 12:
            continue
        regions.append(image.crop((left, top, right, bottom)).convert("RGB"))
    return regions


def run_handwriting(image_path: Path) -> str:
    import torch
    from PIL import Image
    from transformers import TrOCRProcessor, VisionEncoderDecoderModel

    torch.set_num_threads(max(1, min(4, os.cpu_count() or 1)))
    torch.set_num_interop_threads(1)
    model_name = os.environ.get(
        "HYDROGEN_TROCR_MODEL", "microsoft/trocr-small-handwritten"
    )
    processor = TrOCRProcessor.from_pretrained(model_name)
    model = VisionEncoderDecoderModel.from_pretrained(model_name)
    model.to("cpu")
    model.eval()

    with Image.open(image_path) as source:
        lines = _line_regions(source.convert("RGB"))

    output = []
    with torch.inference_mode():
        for line in lines:
            pixel_values = processor(
                images=line, return_tensors="pt"
            ).pixel_values
            generated = model.generate(
                pixel_values,
                max_new_tokens=128,
                num_beams=1,
            )
            text = processor.batch_decode(
                generated, skip_special_tokens=True
            )[0].strip()
            if text:
                output.append(text)
    return "\n\n".join(output)


def run_rapid(image_path: Path) -> str:
    from rapidocr_onnxruntime import RapidOCR

    result, _ = RapidOCR()(str(image_path))
    if not result:
        return ""
    return "\n".join(
        str(item[1]).strip()
        for item in result
        if item and len(item) >= 2 and str(item[1]).strip()
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", required=True, type=Path)
    parser.add_argument(
        "--engine",
        choices=("auto", "math", "handwriting", "rapid"),
        default="auto",
    )
    args = parser.parse_args()

    if not args.image.is_file():
        raise FileNotFoundError(f"Image not found: {args.image}")

    engine = "math" if args.engine == "auto" else args.engine
    if engine == "math":
        text = run_math(args.image)
    elif engine == "handwriting":
        text = run_handwriting(args.image)
    else:
        text = run_rapid(args.image)

    emit_markdown(text, args.image)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"Hydrogen OCR: {exc}", file=sys.stderr)
        raise SystemExit(1)
