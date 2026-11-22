# Personal knowledge, web capture and frontend

Hydrogen's personal knowledge features are local-first. The Rust application owns the SQLite database, retrieval, Ollama requests and configuration. The bundled frontend and browser extension are clients of the local service. These features are independent of Quickshell or any other desktop shell.

## Start the local frontend

~~~sh
hydrogen --frontend
hydrogen --frontend --ask "What is a throttling device?"
hydrogen --frontend --conversation CONVERSATION_ID
hydrogen --frontend --page WEB_PAGE_ID
hydrogen search "throttling device"
~~~

The frontend service binds to the loopback address configured by `frontend.bind_address` (default `127.0.0.1:17891`). When `frontend.auto_start` is enabled, the frontend command starts the service in the background if it is not already running, then opens the local web application.

Run the service in the foreground for debugging:

~~~sh
hydrogen serve
~~~

## Answer from local sources

~~~sh
hydrogen answer "What is a throttling device?"
hydrogen answer --browser "What is a throttling device?"
~~~

Hydrogen retrieves passages from indexed files and captured web articles, sends bounded context to the configured Ollama model, and stores the answer with its source references. The browser option writes a local HTML answer and opens it. The frontend uses the same answer pipeline while keeping messages in the active conversation.

Answer generation is explicit. Searching or browsing saved knowledge does not invoke Ollama automatically. The chat interface supports multi-turn conversations, regeneration, retrying failed answers, copying responses and incremental output updates. A per-message source-grounding control enables or disables the source-grounded answer mode. Disabling grounding is not a guarantee of factual accuracy.

## Browser extension

Hydrogen Capture is a Manifest V3 WebExtension for Firefox and Chromium-based browsers. Its source is in `browser-extension/`.

Build both packages from the repository root:

~~~sh
make extension-package
~~~

Or choose an output directory:

~~~sh
bash scripts/package-browser-extension.sh ~/Downloads/hydrogen-extensions
~~~

The script validates browser manifests and referenced files, stages isolated browser-specific copies, and creates:
- `browser-extension/dist/hydrogen-capture-firefox.xpi`
- `browser-extension/dist/hydrogen-capture-chrome.zip`

Source manifests are not modified. The Firefox XPI is suitable for temporary installation; normal Firefox distribution requires Mozilla signing. The Chrome ZIP is suitable for Web Store upload. For local Chromium testing, extract it and load the directory as an unpacked extension. See [the extension README](../browser-extension/README.md).

Capture features include:
- Mozilla Readability extraction with a structural fallback.
- Canonical URL deduplication and minimum text/paragraph qualification.
- Explicit manual page capture.
- Best-effort rendered conversation adapters for supported AI chat websites.
- Incremental conversation snapshots keyed by conversation URL.
- Independent pause, article capture and AI chat capture controls.
- Explicit recent browser-history import, limited to current content when each page is opened.
- Domain exclusions and URL-pattern rules enforced by the Hydrogen service.
- Incognito/private tabs are not captured.

The extension sends captured content to the local Hydrogen API. It does not bypass site authentication or access hidden model reasoning, private API data or content absent from the rendered page. Website DOM changes can break individual adapters, so provider coverage is best-effort.

## Pairing and API security

The extension must be paired with the local Hydrogen service using its API token. Treat the token as a password: anyone who obtains it can access protected API routes.

The service binds to loopback by default. The health endpoint is available for basic service checks; protected API routes require bearer authentication. Do not expose the service to a LAN or public network without reviewing the bind address, authentication and network security implications.

## Configuration

The TOML file remains the shared source of truth: `~/.config/hydrogen/config.toml`.

Relevant groups:
- `[frontend]`: local bind address and automatic service startup.
- `[web]`: automatic article capture, qualification, page versions and domain/URL rules.
- `[conversations]`: message indexing and browser conversation capture.
- `[answer]`: local model endpoint, selected model and generation/retrieval limits.

The frontend Settings page edits supported values through the validated configuration API. Changes are reloaded between API requests. Changing the listening address requires restarting the service.

## Storage

The local SQLite database stores file metadata and extracted content alongside:
- PDF page text and OCR metadata.
- Text and image embeddings when semantic indexing is enabled.
- Conversations and messages, including saved answer source references.
- FTS indexes for document and conversation search.
- Web pages, visit history and page versions.
- Stable source keys for captured browser conversations.

Schema migrations are additive so existing indexes can be upgraded without discarding indexed content.

## Current implementation boundaries

- RSS/Atom subscription management and polling are not implemented.
- Browser history import captures current page content when it can still be loaded; it does not reconstruct historical snapshots.
- Generic article extraction may need site-specific rules for unusual pages.
- AI-chat adapters depend on rendered website DOM and need regression fixtures as providers change.
- Attachment capture and offline attachment storage are not implemented.
- A full background ingestion review queue and detailed activity monitor remain future work.
- Some advanced configuration remains TOML/CLI-only.
- The extension cannot capture private content that the browser does not expose to content scripts.

The default service is loopback-only. The frontend and extension are clients of the same local service; neither requires a Quickshell runtime.
