# Hydrogen
Hydrogen is a local-first Rust indexer and hybrid search backend for Quickshell.

## Build
```sh
cargo build --release
cargo test
make install
```

## Index and search
```sh
hydrogen index sync
hydrogen index embed                 # backfill existing text and image vectors
hydrogen query "heat exchanger" --semantic
hydrogen query --image ./diagram.png
hydrogen doctor
```

Semantic model downloads are opt-in. Set `HYDROGEN_EMBEDDINGS=1` before
`hydrogen index sync` to keep vectors updated during future syncs. The first
semantic command downloads its selected model files; later inference runs locally.

## Local models
Set `HYDROGEN_MODELS_DIR` (default `~/.local/share/hydrogen/models`).
See [docs/MODELS.md](docs/MODELS.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
Hydrogen OCR no longer invokes Python: printed OCR uses the Rust
`pure-onnx-ocr` pipeline and mathematical crops use Texo OCR. Run
`hydrogen ocr file IMAGE` for printed text or
`hydrogen ocr file IMAGE --math` for a formula crop.

Optional free-form image captioning is built separately with
`cargo build --release --features image-captioning` and enabled at runtime with
`HYDROGEN_CAPTIONS=1`. It is disabled by default because its memory footprint
is larger than the OCR and embedding models.

## Licence
Hydrogen is distributed under AGPL-3.0-only. See [LICENSE](LICENSE).
Model weights remain subject to their own upstream licences.
