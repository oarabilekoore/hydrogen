# Hydrogen architecture

Hydrogen is a local-first knowledge engine implemented as a Rust application. It works independently of any desktop shell or browser. The CLI, local HTTP API, bundled web frontend, browser extension and optional desktop integrations are different entry points into the same local data and retrieval system.

## System overview

~~~text
                 Filesystem roots
                        |
             Scanner / file watchers
                        |
        Extraction: text, PDF pages, OCR
                        |
                 SQLite index
                        |
       +----------------+----------------+
       |                |                |
   CLI search       Local HTTP API   Background jobs
       |                |                |
       |          Web frontend     Browser extension
       |                |                |
       +---------- Retrieval / RAG -------+
                        |
              Local Ollama endpoint
~~~

The diagram is conceptual. The CLI, server and background workers run in Hydrogen or child processes depending on the command. The browser extension is a separate client that communicates with the local API.

## Main components

- **CLI:** configures roots, syncs the index, runs search and OCR, generates answers and reports diagnostics.
- **Indexing pipeline:** scans configured filesystem roots, extracts supported text and PDF content, records content hashes and updates SQLite.
- **Multimodal pipeline:** runs printed-text OCR, formula recognition, optional image captioning and optional semantic embedding generation.
- **SQLite storage:** stores indexed documents, page-level content, vectors, web pages and visits, conversations, messages, source references and background job state.
- **Retrieval:** combines filename/path matching, SQLite FTS5/BM25, sparse TF-IDF/math tokens and optional MiniLM/CLIP vector retrieval.
- **Answer generation:** retrieves local passages and sends bounded context to the configured Ollama model. Generation is explicit and is not triggered by ordinary search.
- **HTTP API and frontend:** expose local search, history, conversations, capture rules, model selection and chat workflows through a loopback service and bundled web UI.
- **Browser extension:** extracts readable articles and supported rendered AI conversations, then sends captures to the local service.
- **Integrations:** desktop launchers and other applications can invoke the CLI or use the API. Quickshell is an example integration, not a dependency.

## Local-first data flow

Filesystem content is indexed on the machine where Hydrogen runs. Browser content is captured by the extension when the user enables capture or requests a manual capture. The extension posts content to the configured local Hydrogen endpoint.

Hydrogen stores its index in SQLite and keeps model files below `HYDROGEN_MODELS_DIR` (default `~/.local/share/hydrogen/models`). Ollama is a separate local service. The default workflow does not require a hosted Hydrogen account or cloud inference endpoint.

The HTTP service binds to loopback by default. API write routes require authentication, and browser extension pairing uses a local API token. Treat that token as a credential: anyone who obtains it can call protected API routes.

## Runtime and resource budget

The default CPU inference budget is two threads and one small image batch at a time. Image indexing runs at low process priority. PDF extraction is bounded by the configured page limit, and previews are rendered only for scanned-page OCR or final visible PDF search results. Worker duration and resident memory are recorded for diagnostics.

Model objects remain resident until their process exits. One-shot commands such as `hydrogen index embed` naturally release them on exit. A long-running daemon retains models it has loaded. Semantic indexing is opt-in because it adds model downloads, CPU work and memory pressure.

## Modalities

- **Printed text:** PP-OCR detector and recognizer through the Rust `pure-onnx-ocr` pipeline.
- **Equations:** Texo OCR, a Rust-native formula recognizer that accepts formula crops and emits LaTeX. It is not a page-layout OCR engine.
- **Handwriting:** mathematical expressions are supported by Texo, but Hydrogen does not claim a dedicated handwritten-prose recognizer.
- **PDFs:** extract embedded text first. Image-only pages are rendered and sent through OCR. Page number, text and preview metadata are stored separately from the parent file.
- **Images:** OCR, optional captions and visual embeddings are separate channels.
- **Text vectors:** quantized MiniLM embeddings for document passages.
- **Cross-modal vectors:** CLIP ViT-B/32 text and image encoders share a space, allowing text queries to retrieve visually relevant images.

## Index integrity

Hydrogen uses BLAKE3 content hashes rather than relying only on file size and modification time. Filesystem events trigger incremental jobs; periodic root reconciliation repairs dropped events. Deletes remove related page, embedding, FTS and cache rows transactionally. New files can become lexically searchable before expensive OCR or embedding work finishes.

## Compatibility and integration boundary

SQLite schema changes are additive migrations. Existing document rows remain valid as page, vector, web and conversation tables are introduced. Rebuild vector data with `hydrogen index embed`; lexical search remains available while optional model files are missing or inference fails.

Hydrogen does not require Quickshell, Hyprland, Wayland or a particular browser. A client can use the CLI for one-shot operations or the local HTTP API for interactive workflows. Desktop-specific behaviours belong in the integration layer, not in the core indexing and retrieval engine.
