# Adapter layer

Adapters connect Hydrogen's application workflows to external systems. This is where filesystem APIs, SQLite, ONNX Runtime, OCR engines, and operating-system facilities belong.

## Design rule

Keep a stable application-facing contract and hide implementation-specific details behind it. The domain layer should not know whether a file watcher is inotify, FSEvents, or ReadDirectoryChangesW.

## Rust references

Inspect all modules under `src/adapters/`. The initial Odin tree focuses on filesystem, SQLite, and ML adapters because those are the core dependencies for indexing and retrieval.

## Candidate integrations

- SQLite C API for database operations.
- BLAKE3 for content hashing.
- ONNX Runtime C API for model inference.
- Native OS APIs for filesystem watching.
- Existing command-line extractors where a robust native library is not yet justified.

Evaluate licensing, deployment size, static/dynamic linking, ABI stability, platform support, and error reporting before adopting a dependency.

Resources: [Odin foreign system](https://odin-lang.org/docs/overview/#foreign-system), [SQLite C API](https://www.sqlite.org/c3ref/intro.html), [ONNX Runtime C API](https://onnxruntime.ai/docs/api/c/).
