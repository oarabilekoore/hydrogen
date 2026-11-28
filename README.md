# Good Search

Good Search is a local-first knowledge and retrieval system written in Rust. It indexes files and readable web content, extracts text from documents and images, provides hybrid search, and answers questions from retrieved sources using a local language model.

Good Search is a standalone application with several entry points:
- **CLI** for indexing, search, OCR, diagnostics, configuration and source-grounded answers.
- **Local HTTP service and web frontend** for browsing saved knowledge, conversations, history and settings.
- **Browser extension** for capturing readable articles and supported AI conversations.
- **Integrations** for desktop launchers and other applications. Quickshell is one possible integration, not a runtime requirement.

## Capabilities

| Area | What it does |
| --- | --- |
| File indexing | Watches and synchronizes configured directories with incremental updates and content hashing |
| Document extraction | Extracts text from supported files and indexes PDF pages |
| Image understanding | Runs printed-text OCR and formula recognition; optional captions and embeddings |
| Search | Combines lexical/FTS search with optional text and image semantic retrieval |
| Personal knowledge | Stores captured articles, browser visits, conversations and answers in SQLite |
| Local RAG | Retrieves relevant passages and asks a configured Ollama model to answer with source references |
| Browser capture | Captures readable pages and supported rendered AI conversations |
| API and frontend | Exposes a loopback HTTP API and bundled web interface |
| Integrations | Provides CLI/API entry points for launchers and other clients |

## Quick start

### Requirements

- Rust toolchain and Cargo to build from source.
- Linux is the primary development and runtime target.
- `make`, `zip`, `unzip` and Python 3 for the browser-extension packaging helper.
- Ollama is optional and required only for local AI answer generation.
- OCR and semantic models are downloaded separately when their features are used.

### Build and install

~~~sh
cargo build --release
cargo test
make install
~~~

The default install target is `~/.local/bin/gsearch`. Ensure that directory is on your `PATH`.

To run from the checkout:

~~~sh
cargo run -- --help
cargo run -- index sync
~~~

### Configure indexed directories

~~~sh
gsearch config init
$EDITOR ~/.config/gsearch/config.toml
~~~

Add directories under `[paths].roots`. Inspect effective settings with `gsearch config show`. See [Configuration](docs/CONFIGURATION.md).

### Index and search

~~~sh
gsearch index sync
gsearch search "heat exchanger"
gsearch query "throttling device"
gsearch doctor
~~~

Use `gsearch search` for broad cross-source search, including files, saved web pages, conversations and answers. Use `gsearch query` for document and multimodal retrieval.

### OCR and semantic search

~~~sh
make models
gsearch ocr file ./scan.png
gsearch ocr file ./equation.png --math
gsearch index embed
gsearch query "heat exchanger" --semantic
gsearch query --image ./diagram.png
~~~

Semantic indexing is opt-in. Enable it in TOML before syncing if future syncs should maintain vectors. First use downloads model files; inference runs locally afterward. See [Model setup](docs/MODELS.md).

### Local AI answers

~~~sh
ollama pull qwen3.5:2b-q4_K_M
gsearch answer "Explain the throttling process"
~~~

Good Search retrieves local passages and asks Ollama to answer from that context. Answer generation is explicit; ordinary search does not invoke a language model. See [RAG answers](docs/RAG.md).

### Web frontend and browser capture

~~~sh
gsearch --frontend
make extension-package
# Or choose an output directory:
bash scripts/package-browser-extension.sh ~/Downloads/gsearch-extensions
~~~

The frontend is served locally. The extension package command creates a Firefox `.xpi` and Chrome-compatible `.zip`. Firefox normally requires Mozilla signing for permanent distribution. For local development, use Firefox's temporary add-on loader or extract the Chrome archive and load it unpacked. See the [extension guide](browser-extension/README.md).

## Common commands

| Command | Purpose |
| --- | --- |
| `gsearch index sync` | Synchronize configured file roots |
| `gsearch index embed` | Backfill semantic vectors |
| `gsearch search QUERY` | Search across indexed knowledge sources |
| `gsearch query QUERY` | Search documents and multimodal content |
| `gsearch query --image FILE` | Search with an image query |
| `gsearch ocr file FILE` | Extract printed text from an image |
| `gsearch answer QUESTION` | Generate a source-grounded local answer |
| `gsearch serve` | Run the local HTTP service in the foreground |
| `gsearch --frontend` | Start/open the bundled web frontend |
| `gsearch doctor` | Inspect index, model and runtime health |
| `gsearch config show` | Show effective configuration |

Use `gsearch --help` and `gsearch COMMAND --help` for the authoritative command and option list for the installed build.

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Configuration](docs/CONFIGURATION.md)
- [Local model setup](docs/MODELS.md)
- [Personal knowledge frontend and browser capture](docs/PERSONAL_KNOWLEDGE.md)
- [Local RAG answers](docs/RAG.md)
- [Browser extension build and installation](browser-extension/README.md)

## Project status and boundaries

Good Search is under active development. Extraction quality varies by file format and website. AI-chat capture depends on rendered page structure and may need adapter updates as sites change. OCR and semantic retrieval depend on separately installed model files. Source-grounded answers can still be incomplete or incorrect if retrieval misses relevant material or the model misinterprets it.

The service binds to loopback by default. Do not expose it to an untrusted network without reviewing the bind address, authentication and network security boundary.

## License

Good Search is distributed under AGPL-3.0-only. See [LICENSE](LICENSE). Model weights and bundled third-party components retain their upstream licenses.
