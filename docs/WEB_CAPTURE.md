# Hydrogen Capture

Hydrogen Capture is a browser extension bundled in `browser-extension/`.

## Install in Firefox

1. Open `about:debugging#/runtime/this-firefox`.
2. Select **Load Temporary Add-on**.
3. Select `browser-extension/manifest.json`.
4. Open the extension's options and verify the Hydrogen address is `http://127.0.0.1:17843`.
5. Start Hydrogen with `hydrogen --frontend`.

For Chromium-based browsers, load the `browser-extension` directory from `chrome://extensions` with Developer mode enabled.

Temporary Firefox add-ons must be loaded again after the browser restarts. A signed release package is a separate distribution task.

## Capture behaviour

The extension currently supports:

- Readable article capture from pages with an `article` or `main` content region.
- Automatic qualification based on readable text and paragraph count.
- Manual **Save this page** from the extension popup.
- Visible conversation capture for ChatGPT, Gemini and Claude.
- Incremental conversation updates, deduplicated in the local database.

It intentionally does not capture cookies, passwords, form values or browser storage. It captures visible rendered conversation messages, not private model reasoning or hidden server-side content.

## Qualification and domain rules

Hydrogen applies the central configuration:

- `web.enabled`
- `web.auto_index`
- `web.minimum_text_length`
- `web.minimum_paragraphs`
- `web.allow_domains`
- `web.exclude_domains`
- `web.exclude_patterns`
- `web.max_page_size_mb`

An empty allow-list permits any page that passes qualification. Excluded domains and URL patterns are always skipped. Manual saving bypasses the text-length and paragraph thresholds, but not explicit domain exclusions.

## Troubleshooting

- Make sure Hydrogen is running and the configured port matches the extension.
- Reload the page after installing or updating the extension so the content script is injected.
- Check that the page contains an article or main content region.
- Review the web qualification settings in Hydrogen's frontend.
