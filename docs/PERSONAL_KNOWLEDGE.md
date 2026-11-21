# Personal Knowledge Engine

Hydrogen's personal knowledge features are local-first. The Rust binary owns the SQLite database, retrieval, Ollama requests and configuration. The bundled frontend and browser extension are clients of that local service.

## Open the frontend

```sh
hydrogen --frontend
hydrogen --frontend --ask "What is a throttling device?"
hydrogen --frontend --conversation CONVERSATION_ID
hydrogen --frontend --page WEB_PAGE_ID
hydrogen search "throttling device"
```

The frontend service binds to the loopback address configured by `frontend.bind_address` (default `127.0.0.1:17891`). The first command starts the service in the background when `frontend.auto_start` is enabled, then opens the local web application.

To run the service in the foreground for debugging:

```sh
hydrogen serve
```

## Answer in a browser

```sh
hydrogen answer --browser "What is a throttling device?"
```

The command retrieves passages from indexed files and captured web articles, asks the configured Ollama model for a source-grounded answer, saves a conversation with its source references, writes a standalone HTML answer and opens it in the default browser.

The frontend uses the same answer pipeline but saves its messages into the active conversation rather than creating a second conversation.

## Browser extension

Load the unpacked `browser-extension/` directory in Chromium or Firefox.

Hydrogen Capture currently supports:

- Readable article capture from article/main content containers.
- Canonical URL deduplication.
- Minimum text and paragraph qualification.
- Explicit manual page capture.
- Best-effort rendered conversation adapters for ChatGPT, Claude, Gemini, Perplexity, DeepSeek, Microsoft Copilot, Grok, Poe and Mistral.
- Incremental conversation snapshots keyed by their conversation URL.
- Independent pause, article capture and AI chat capture controls.
- Explicit recent browser-history import (last 30 days, up to 25 URLs by default), which visits selected URLs in inactive tabs and captures only qualifying readable articles.

The extension sends content to the local Hydrogen API only. The API enforces the configured domain exclusions, allowed-domain list, content-size limit and automatic-capture qualification.

## Configuration

The existing TOML file remains the single source of truth:

`~/.config/hydrogen/config.toml`

New groups:

- `[frontend]`: local bind address and automatic service startup.
- `[web]`: automatic article capture, qualification, page versions, RSS polling preferences and domain/URL rules.
- `[conversations]`: message indexing and browser conversation capture.

The frontend Settings page edits supported values through `hydrogen config set`. Changes are reloaded between API requests. Changing the listening address requires restarting the service.

## Storage

The existing SQLite index is upgraded additively to schema version 7. It retains the file, PDF page, image OCR and embedding tables and adds:

- `conversations` and `messages`, including saved answer source JSON.
- FTS5 indexing for conversation messages.
- `web_pages` and `web_pages_fts`.
- `web_visits` and `web_page_versions`.
- A stable source-key map for captured browser conversations.

## Current implementation boundaries

This is an initial working foundation, not the completed roadmap. The following are still follow-up work:

- RSS/Atom feed subscription management and polling.
- Full historical reconstruction of pages from browser history. Import can only capture the current version of a page that still loads.
- Mozilla Readability-grade extraction and site-specific extraction fixtures.
- Robust DOM regression fixtures for the current best-effort AI chat adapters and any additional providers.
- Attachment capture and offline attachment storage.
- Background ingestion queues, retry UI and detailed activity monitoring.
- Settings controls for supported writable options; a few advanced or not-yet-implemented settings remain CLI/TOML-only.
- Streaming model output and explicit service lifecycle commands.
- End-to-end runtime validation on a Linux desktop with the user's Ollama installation.

The service binds to loopback only. It does not expose the API on the LAN.
