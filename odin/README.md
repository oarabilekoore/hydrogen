# Hydrogen Odin Rewrite

This directory is the planned Odin implementation of Hydrogen. The existing Rust implementation remains the behavioural reference until the Odin version can pass the same compatibility tests.

## Goals

- Learn Odin by reimplementing Hydrogen subsystem by subsystem.
- Preserve Hydrogen's externally visible behaviour and existing data where practical.
- Keep the frontend and browser extension independent of the core-language rewrite.
- Make platform-specific behaviour replaceable so Linux, Windows, and macOS can be supported.
- Measure executable size, runtime dependencies, model assets, and total installation size separately.

## Relationship to Rust

The layout under `src/` intentionally follows the current Rust `src/` layout. Names are starting points, not a promise that every Rust module should remain one Odin file. Split large Rust services into smaller Odin packages or files when the responsibilities become clear.

| Rust reference | Odin starting point |
|---|---|
| `src/main.rs` | `src/main.odin` |
| `src/config.rs` | `src/config.odin` |
| `src/domain/` | `src/domain/` |
| `src/application/` | `src/application/` |
| `src/adapters/` | `src/adapters/` |
| `src/cli/` | `src/cli/` |

Do not translate syntax line by line. First identify each module's responsibilities, data contracts, side effects, invariants, and failure cases. Then design the Odin equivalent.

## Recommended implementation order

1. Application entry point, configuration, logging, and diagnostics.
2. SQLite connection and schema compatibility.
3. Domain data structures and repository contracts.
4. Filesystem scanning, metadata, hashing, and exclusions.
5. Text extraction and document normalization.
6. Lexical search and ranking.
7. Incremental indexing and reliability.
8. OCR and formula recognition.
9. Embeddings and semantic retrieval.
10. RAG and Ollama integration.
11. HTTP API and CLI parity.
12. Personal-knowledge integrations and cross-platform release.

Keep each milestone independently testable. A usable lexical search engine is more valuable than a half-implemented AI pipeline.

## Odin learning resources

- [Official Odin documentation](https://odin-lang.org/docs/)
- [Language overview](https://odin-lang.org/docs/overview/)
- [Getting started and compiler installation](https://odin-lang.org/docs/install/)
- [Package API reference](https://pkg.odin-lang.org/)
- [Official Odin repository and examples](https://github.com/odin-lang/Odin)

Odin is directory-package-oriented. Files in one directory form one package and must use the same package declaration. Read the official package and allocator documentation before designing module boundaries or memory ownership.

## Candidate libraries and native interfaces

Treat these as candidates to evaluate, not dependencies to add automatically:

- SQLite: bind to the SQLite C API. Preserve the current schema initially; write explicit migrations and test against a copied database.
- Hashing: evaluate BLAKE3's C API or a small audited implementation. Match the Rust implementation's exact digest format.
- ONNX Runtime: use its C API for OCR and embedding inference if model/operator compatibility is confirmed.
- HTTP: inspect Odin's `core:net` packages and available vendor bindings before selecting a client/server design.
- Filesystem watching: use separate Linux inotify, Windows ReadDirectoryChangesW, and macOS FSEvents adapters.
- Document extraction: compare native libraries and command-line tools per format. Keep extractors behind a stable interface.
- Ollama: use its HTTP API; avoid tying the domain layer to a specific local model provider.

Useful starting points include [SQLite's C API](https://www.sqlite.org/c3ref/intro.html), [ONNX Runtime C API](https://onnxruntime.ai/docs/api/c/), [BLAKE3](https://github.com/BLAKE3-team/BLAKE3), and [Odin's foreign system](https://odin-lang.org/docs/overview/#foreign-system).

## Compatibility discipline

Build a shared fixture corpus and run it through both implementations. Compare:

- Indexed file identity, hashes, timestamps, and deletion state.
- Extracted text, page numbers, metadata, and OCR output.
- Search results, ranking, snippets, and source references.
- Database integrity and reopen behaviour.
- Retry behaviour after interrupted or failed jobs.
- API response shapes and streaming behaviour.

Keep a copy of the Rust database for migration and compatibility tests. Never use the only live database as a test fixture.

## Important design cautions

- Odin uses explicit memory management and allocators. Decide who owns every allocated buffer and when it is released.
- Do not let filesystem paths, watcher APIs, or desktop integrations leak into domain logic.
- Do not assume replacing Rust with Odin automatically reduces the installed footprint. Measure the executable, linked libraries, model files, and total package separately.
- Keep model identifiers, dimensions, preprocessing, normalization, and embedding metadata stable when comparing results.
- The formula OCR dependency is Rust-specific today. Treat it as a separate replacement or temporary interoperability problem rather than assuming a GGUF file alone provides inference.

## Working method

For each module: inspect the Rust implementation; write down its public inputs and outputs; identify invariants and error paths; design the Odin data types; implement the smallest useful slice; add tests; compare behaviour with Rust; then move to the next responsibility.

The stub files are deliberately unfinished. Their purpose is to give the rewrite a map, not to hide the learning behind generated code.
