# Hydrogen Capture

A Manifest V3 extension for Chromium and Firefox-compatible WebExtension environments.

## Development install

- Chromium: open `chrome://extensions`, enable Developer mode, choose **Load unpacked**, and select this directory.
- Firefox: open `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on**, and select `manifest.json`.

## Behaviour

- Uses the bundled Mozilla Readability library to extract article text, metadata and paragraph structure, with a structural DOM fallback.
- Uses minimum text and paragraph thresholds to avoid indexing app interfaces.
- Captures rendered ChatGPT, Claude and Gemini conversation turns where their DOM exposes them.
- Sends captured content only to the Hydrogen service at `127.0.0.1:17891`.
- Uses canonical URLs for article deduplication and a stable conversation URL for snapshot replacement.
- Supports pausing capture and disabling articles or AI chats separately.
- Supports user-managed excluded hostnames, including subdomains, from the extension popup.
- Can explicitly import a limited set of recent browser-history URLs by opening them in inactive tabs one at a time; only pages passing the normal article qualification rules are saved.

## Limitations

This is an initial adapter implementation. AI websites change their DOM frequently, so adapters are best-effort and need fixtures and regression tests. Unknown or unlabelled message nodes are ignored rather than guessed. The extension can only capture content present in the rendered page; it cannot access hidden model reasoning, private API data, or content that the website does not expose.

RSS/Atom feed subscription management, attachment capture, a review queue and historical versions of pages before they are revisited are not implemented yet. The bundled Readability source is Apache-2.0 licensed; its license is included in `vendor/READABILITY-LICENSE.txt`. History import only fetches the current page content available at import time, not a historical snapshot.
