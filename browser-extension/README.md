# Hydrogen Capture

A Manifest V3 extension for Chromium and Firefox-compatible WebExtension environments.

## Development install

- Chromium: open `chrome://extensions`, enable Developer mode, choose **Load unpacked**, and select this directory.
- Firefox: open `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on**, and select `manifest.json`.

## Behaviour

- Extracts readable article text from article/main content containers.
- Uses minimum text and paragraph thresholds to avoid indexing app interfaces.
- Captures rendered ChatGPT, Claude and Gemini conversation turns where their DOM exposes them.
- Sends captured content only to the Hydrogen service at `127.0.0.1:17891`.
- Uses canonical URLs for article deduplication and a stable conversation URL for snapshot replacement.
- Supports pausing capture and disabling articles or AI chats separately.

## Limitations

This is an initial adapter implementation. AI websites change their DOM frequently, so each adapter needs fixtures and regression tests. The extension can only capture content present in the rendered page; it cannot access hidden model reasoning, private API data, or content that the website does not expose.

Automatic history import, RSS discovery, attachment capture and a review queue are not implemented yet.
