# Good Search Capture

A Manifest V3 extension for Chromium and Firefox-compatible WebExtension environments. The two browsers use different Manifest V3 background declarations, so this directory includes a browser-specific manifest for each.

## Development install

- **Firefox:** open `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on**, and select `manifest.json` in this directory. This manifest uses Firefox's `background.scripts` entry.
- **Chromium:** open `chrome://extensions`, enable Developer mode, choose **Load unpacked**, and select this directory. If Chromium does not accept Firefox's background declaration, use the Chromium manifest variant by temporarily copying `manifest.chromium.json` to `manifest.json` before loading the directory. Do not leave both manifest files named `manifest.json` at once.

The extension source files and background implementation are shared. Only the manifest background declaration differs: Firefox uses `background.scripts`, while Chromium Manifest V3 uses `background.service_worker`.

## Build installable packages

From the repository root, run:

```sh
make extension-package
# or:
bash scripts/package-browser-extension.sh
```

The script validates both Manifest V3 manifests and the files they reference, then writes:

- `browser-extension/dist/gsearch-capture-firefox.xpi`
- `browser-extension/dist/gsearch-capture-chrome.zip`

The source manifests are never modified. The Firefox XPI can be loaded temporarily from
`about:debugging#/runtime/this-firefox`. Normal Firefox distribution requires Mozilla
signing. The Chrome ZIP is intended for Chrome Web Store submission; for local testing,
extract it and choose **Load unpacked** at `chrome://extensions`.

Pass an output directory as the first argument to the script to change the destination:

```sh
bash scripts/package-browser-extension.sh ~/Downloads/gsearch-extensions
```

## Pairing with Good Search

1. Start Good Search once so it creates its local API token, or run `gsearch --print-api-token` to create and print it directly.
2. Copy the token printed by the command. The token is stored in Good Search's state directory in a file with owner-only permissions.
3. Open the extension popup, paste the token into **API token**, and select **Pair with Good Search**. The extension validates the token against the configured service URL before saving it in extension-local storage.
4. Keep the token private. Anyone who has it can access the local Good Search API. To revoke access, stop Good Search, delete `gsearch-api-token` from its state directory, then start Good Search to generate a replacement and pair the extension again.

The bundled Good Search web frontend receives its token from the local server at page-render time. API routes require bearer authentication; `/api/health` remains available without authentication for basic service checks.

## Website capture rules

Good Search applies server-side allow lists, excluded domains and URL glob patterns to every article capture, including manual captures. Empty exclusion patterns are ignored. The extension refuses to capture private/incognito tabs. Use **Ignore this site** in the extension popup to add the active website domain and its subdomains to Good Search's persistent exclusion list. The web settings page includes a URL rule tester. Article qualification thresholds (minimum text length, paragraph count and maximum page size) remain configurable in Good Search settings.

## Behaviour

- Uses the bundled Mozilla Readability library to extract article text, metadata and paragraph structure, with a structural DOM fallback.
- Uses minimum text and paragraph thresholds to avoid indexing app interfaces.
- Captures rendered ChatGPT, Claude and Gemini conversation turns where their DOM exposes them.
- Sends captured content only to the Good Search service at `127.0.0.1:17891`.
- Uses canonical URLs for article deduplication and a stable conversation URL for snapshot replacement.
- Supports pausing capture and disabling articles or AI chats separately.
- Supports user-managed excluded hostnames, including subdomains, from the extension popup.
- Can explicitly import a limited set of recent browser-history URLs by opening them in inactive tabs one at a time; only pages passing the normal article qualification rules are saved.

## Limitations

This is an initial adapter implementation. AI websites change their DOM frequently, so adapters are best-effort and need fixtures and regression tests. Unknown or unlabelled message nodes are ignored rather than guessed. The extension can only capture content present in the rendered page; it cannot access hidden model reasoning, private API data, or content that the website does not expose.

RSS/Atom feed subscription management, attachment capture, a review queue and historical versions of pages before they are revisited are not implemented yet. The bundled Readability source is Apache-2.0 licensed; its license is included in `vendor/READABILITY-LICENSE.txt`. History import only fetches the current page content available at import time, not a historical snapshot.
