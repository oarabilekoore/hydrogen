# Hydrogen

Hydrogen is the native Rust backend for the Quickshell desktop environment. It provides the command-line services used by the launcher: file indexing, content search, image indexing, OCR, and calendar integration.

## Features

- **File indexing:** scans configured user directories and maintains a searchable local index.
- **Content search:** combines SQLite FTS5 text search with a lightweight vector index.
- **Image indexing:** supports OCR text and image-caption metadata for image search.
- **Clipboard and file OCR:** reads image entries from `cliphist` or an image path, extracts text, copies Markdown to the Wayland clipboard, and can paste into the previously focused Hyprland window.
- **Calendar:** lists, creates, and launches desktop calendar events.
- **Daemon:** runs the background watcher and incremental indexer.

## Architecture

The project uses a layered Rust structure:

- `src/domain/` contains domain models and OCR layout formatting.
- `src/application/` contains application services, including OCR orchestration.
- `src/ports/` defines interfaces for OCR, desktop integration, storage, and extraction.
- `src/adapters/` contains the concrete ML, filesystem, SQLite, calendar, and Wayland adapters.
- `src/cli/` exposes the command-line interface.

OCR is deliberately invoked through a Python worker so the Rust application can keep a small dependency footprint. The current worker uses RapidOCR from a dedicated virtual environment. The OCR result is converted into Markdown and copied with `wl-copy`.

## Requirements

- Linux with a Wayland session
- Rust toolchain and Cargo
- SQLite (bundled through the Rust dependency)
- `cliphist`, `wl-copy`, `wtype`, `hyprctl`, and `notify-send` for the full clipboard-to-paste workflow
- Python 3 and the RapidOCR runtime environment for OCR

## Build and install

Clone and build:

```sh
git clone https://github.com/oarabilekoore/hydrogen.git
cd hydrogen
make build
make install
```

The default install prefix is `~/.local`, so the binary is installed to `~/.local/bin/hydrogen`. Ensure that directory is on your `PATH`.

Run the test suite:

```sh
make test
```

## OCR runtime

The current configuration expects the RapidOCR Python interpreter at:

```text
~/.local/share/rapidocr-venv/bin/python
```

The Python environment must provide `rapidocr_onnxruntime`. Hydrogen starts the worker on demand; it does not keep a large OCR model loaded in the Rust daemon.

### OCR commands

Recognise an image from a clipboard-history entry:

```sh
hydrogen ocr clip CLIPHIST_ID
```

Recognise an image file:

```sh
hydrogen ocr file ~/Pictures/notes.png
```

To paste the result into a specific Hyprland window, provide its address:

```sh
hydrogen ocr file ~/Pictures/notes.png --window WINDOW_ADDRESS
```

OCR output is copied as plain-text Markdown. When a window address is supplied, Hydrogen focuses that window and sends Ctrl+V.

## Configuration

Hydrogen reads environment variables and derives its default paths from `$HOME`.

| Variable | Purpose |
| --- | --- |
| `HYDROGEN_STATE_DIR` | Override the application state directory. Defaults to `~/.local/state/quickshell-launcher`. |
| `DURATION` | Default calendar-event duration in minutes. Defaults to `60`. |

The default index/search roots are `Documents`, `Downloads`, `Pictures`, `Videos`, and `Projects`.

## User systemd service

Install and enable the background daemon:

```sh
make service
```

This installs the binary and creates `quickshell-hydrogen.service` under the user systemd directory.

Stop and remove the service and installed binary:

```sh
make uninstall
```

## Related project

Hydrogen is designed to work with [dotfiles-2](https://github.com/oarabilekoore/dotfiles-2), which contains the Quickshell launcher and desktop configuration.

## License

No license is currently declared in this repository. Until one is added, assume the source is not licensed for redistribution or reuse beyond the rights granted by GitHub's terms.
