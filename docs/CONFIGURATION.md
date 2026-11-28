# Good Search configuration

Good Search uses one validated TOML configuration for its CLI, daemon, local HTTP service, web frontend and integrations.

The default file is `~/.config/gsearch/config.toml`. Override the location with `GSEARCH_CONFIG`, or set `XDG_CONFIG_HOME` to change the parent configuration directory. If the file does not exist, Good Search uses built-in defaults. It does not write a configuration file automatically.

Create a starter file and inspect effective settings:

~~~sh
gsearch config init
gsearch config show
gsearch config path
~~~

## Example

~~~toml
[paths]
roots = ["~/Documents", "~/Downloads", "~/Pictures", "~/Videos", "~/Projects"]

[search]
default_limit = 25
include_hidden = false
scan_max_depth = 6
scan_max_results = 5000
excluded_directories = [".git", "node_modules", ".cache", "target", "build"]
excluded_extensions = ["tmp", "part", "swp", "lock"]

[indexing]
embeddings_enabled = false

[indexing.pdf]
enabled = true
ocr_scanned_pages = true
page_limit = 500

[indexing.images]
enabled = true
ocr = true
captions = false
min_file_bytes = 10240
max_file_bytes = 26214400
min_dimension = 128
batch_size = 2
excluded_directories = ["assets", "icons", "themes", "thumbnails"]

[models]
inference_threads = 2
ocr_printed = "ppocr"
ocr_formula = "texo-distill-f16.gguf"

[resources]
max_inference_threads = 2

[answer]
provider = "ollama"
endpoint = "http://127.0.0.1:11434"
model = "qwen3.5:2b-q4_K_M"
retrieval_limit = 8
candidate_limit = 16
max_query_variants = 6
min_query_chars = 3
context_chars_per_passage = 1600
context_window = 4096
max_output_tokens = 180
temperature = 0.0
top_p = 0.7
repeat_penalty = 1.08
timeout_seconds = 60

[watch]
debounce_ms = 350
reconcile_interval_seconds = 60
poll_interval_ms = 150
~~~

Paths support normal TOML strings. Use absolute paths or paths relative to the home directory; Good Search expands a leading `~` in configured search roots, state directories and model directories.

## Configuration groups

- `paths.roots`: directories scanned by one-shot sync and the daemon.
- `paths.state_dir` and `paths.models_dir`: index/cache and model storage.
- `search.*`: result count, hidden files, scan depth, scan cap and excluded directory names/extensions.
- `indexing.pdf.*`: PDF page indexing, scanned-page OCR and page limits.
- `indexing.images.*`: standalone image indexing, printed OCR, optional captioning, size/dimension thresholds, batch size and skipped directories.
- `indexing.embeddings_enabled`: whether semantic text and visual embeddings are maintained and used by default search.
- `models.*` and `resources.*`: OCR model paths and CPU inference budget.
- `answer.*`: Ollama endpoint/model, retrieval count, context and generation limits.
- `watch.*`: filesystem event debounce, reconciliation and polling intervals.
- `frontend.*`: local service bind address and automatic frontend service startup.
- `web.*`: article capture, qualification, page versioning and domain/URL rules.
- `conversations.*`: conversation indexing and browser conversation capture.
- `calendar.*`: calendar globs and default event duration.

Good Search does not claim to extract every arbitrary embedded image object from otherwise text-based PDFs. The PDF image setting controls OCR of scanned/image-only pages. Embedded-object extraction would be a separate feature.

## Updating settings

Edit TOML directly, or use the validated setter for common options:

~~~sh
gsearch config set answer.model qwen3.5:2b-q4_K_M
gsearch config set answer.retrieval_limit 8
gsearch config set indexing.pdf.ocr_scanned_pages true
gsearch config set indexing.images.enabled true
gsearch config set indexing.embeddings_enabled true
gsearch config set paths.roots '["/home/me/Documents", "/home/me/Notes"]'
~~~

For list settings, pass a JSON array or a comma-separated list. `config set` writes the configuration file and refuses unknown keys. `config show` displays effective settings after validation and environment overrides.

Legacy environment overrides remain supported for state/model directories, semantic indexing, inference threads, PDF page limits and image captions. TOML is the normal source of truth; environment variables are intended for service-level overrides and temporary testing.

## Client and launcher integration

All clients should use Good Search's own configuration rather than duplicating model names, RAG limits, search limits or indexing environment variables. For example:

- `gsearch query QUESTION`
- `gsearch answer QUESTION`
- `gsearch index sync`
- `gsearch index sync-images`

A desktop launcher may read `gsearch config show` to display effective settings and use `gsearch config set` to change supported values. This is not specific to Quickshell; any shell, launcher or application can follow the same contract.

## Fixed implementation boundaries

Supported file-format lists, SQLite schema details, parser/decompression safety ceilings and low-level OCR engine invariants remain code-owned. They protect correctness and memory use rather than express ordinary user policy. Configurable limits are validated and clamped to safe ranges.
