# Hydrogen
Hydrogen is a local-first Rust indexer and search backend for Quickshell.

## Build
```sh
cargo build --release
cargo test
make install
```

## Local models
Set `HYDROGEN_MODELS_DIR` (default `~/.local/share/hydrogen/models`).
See [docs/MODELS.md](docs/MODELS.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
Hydrogen OCR no longer invokes Python: printed OCR uses the Rust
`pure-onnx-ocr` pipeline and mathematical crops use Texo OCR. Run
`hydrogen ocr file IMAGE` for printed text or
`hydrogen ocr file IMAGE --math` for a formula crop.

## Licence
Hydrogen is distributed under AGPL-3.0-only. See [LICENSE](LICENSE).
Model weights remain subject to their own upstream licences.
