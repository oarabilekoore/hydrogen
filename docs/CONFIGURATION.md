# Hydrogen configuration

Hydrogen is configured centrally through:

`~/.config/hydrogen/config.toml`

Override the location with `HYDROGEN_CONFIG`, or set `XDG_CONFIG_HOME` to
change the parent configuration directory. If the file does not exist,
Hydrogen uses the built-in defaults. It never writes a configuration file
automatically.

Create a complete starter file:

```sh
hydrogen config init
```

Inspect the effective configuration (including environment overrides):

```sh
hydrogen config show
hydrogen config path
```

## Example

```toml
[paths]
roots = [
  "~/Documents",
  "~/Downloads",
  "~/Pictures",
  "~/Videos",
  "~/Projects",
]

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
```

Paths support normal TOML strings. Use absolute paths or paths relative to the
home directory; Hydrogen expands a leading `~` in configured search roots.

## What is configurable

- `paths.roots`: directories scanned by both one-shot sync and the daemon.
- `paths.state_dir` and `paths.models_dir`: index/cache and model storage.
- `search.*`: default result count, hidden files, scan depth, scan cap and
  excluded directory names/extensions.
- `indexing.pdf.enabled`: whether PDF page indexing is enabled.
- `indexing.pdf.ocr_scanned_pages`: render and OCR image-only/scanned PDF pages.
  Enabled by default, so image-only lecture PDFs are not silently ignored.
- `indexing.images.*`: standalone image content indexing, printed OCR, optional
  captioning, size/dimension thresholds, batch size and skipped directories. Disabling
  this turns off image OCR/caption/embedding work; ordinary file metadata remains searchable.
- `indexing.embeddings_enabled`: whether semantic text and visual embeddings
  are maintained and used by default search.
- `models.*` and `resources.*`: OCR model paths and the CPU inference budget.
- `answer.*`: Ollama endpoint/model, retrieval count, context and generation
  limits. `hydrogen answer QUESTION` uses these values; CLI flags can override
  the model and passage count for one invocation.
- `watch.*`: filesystem event debounce, reconciliation and polling intervals.
- `calendar.*`: calendar globs and default event duration.

Hydrogen intentionally does not claim to extract every arbitrary embedded image
object from otherwise text-based PDFs. The PDF image setting controls OCR of
scanned/image-only pages, which is the existing supported path. Embedded-object
extraction would be a separate indexing feature.

## Updating settings

You can edit TOML directly, or use the validated setter for common options:

```sh
hydrogen config set answer.model qwen3.5:2b-q4_K_M
hydrogen config set answer.retrieval_limit 8
hydrogen config set indexing.pdf.ocr_scanned_pages true
hydrogen config set indexing.images.enabled true
hydrogen config set indexing.embeddings_enabled true
hydrogen config set paths.roots '["/home/me/Documents", "/home/me/Notes"]'
```

For list settings, pass a JSON array or a comma-separated list. `config set`
writes the configuration file and refuses unknown keys. `config show` displays
the effective settings after validation and environment overrides.

Legacy environment overrides remain supported for state/model directories,
semantic indexing, inference threads, PDF page limits and image captions.
The TOML file is the normal source of truth; environment variables are intended
for service-level overrides and temporary testing.

## Launcher integration

The Quickshell launcher should call Hydrogen without duplicating model names,
RAG limits, search limits or indexing environment variables:

- `hydrogen query QUESTION`
- `hydrogen answer QUESTION`
- `hydrogen index sync`
- `hydrogen index sync-images`

Those commands use the effective Hydrogen configuration. The launcher reads
`hydrogen config show` to reflect whether semantic indexing is enabled, and
changes that setting through `hydrogen config set`.


## Intentionally fixed implementation boundaries

Not every constant should become a user preference. Supported file-format lists,
SQLite schema details, parser/decompression safety ceilings, and low-level OCR
engine invariants remain code-owned. They protect correctness and memory use
rather than express ordinary user policy. The configurable limits are validated
and clamped to safe ranges. If a new backend or file format is added, it should
extend the schema deliberately instead of adding another unrelated environment
variable or launcher-only constant.
