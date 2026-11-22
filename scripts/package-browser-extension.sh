#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
SOURCE_DIR="$REPO_ROOT/browser-extension"
OUTPUT_DIR="${1:-$SOURCE_DIR/dist}"

for command in cp mktemp python3 rm unzip zip; do
    if ! command -v "$command" >/dev/null 2>&1; then
        printf 'Missing required command: %s\n' "$command" >&2
        exit 1
    fi
done

[[ -f "$SOURCE_DIR/manifest.json" ]] || { echo "Firefox manifest not found." >&2; exit 1; }
[[ -f "$SOURCE_DIR/manifest.chromium.json" ]] || { echo "Chromium manifest not found." >&2; exit 1; }

python3 - "$SOURCE_DIR/manifest.json" "$SOURCE_DIR/manifest.chromium.json" <<'PY'
import json
import pathlib
import sys

for raw_path in sys.argv[1:]:
    path = pathlib.Path(raw_path)
    with path.open(encoding="utf-8") as handle:
        manifest = json.load(handle)
    if manifest.get("manifest_version") != 3:
        raise SystemExit(f"{path}: expected Manifest V3")
    if not manifest.get("name") or not manifest.get("version"):
        raise SystemExit(f"{path}: name and version are required")
    for key in ("background", "action", "content_scripts"):
        if key not in manifest:
            raise SystemExit(f"{path}: missing required manifest key: {key}")
    for script in manifest.get("background", {}).get("scripts", []):
        if not (path.parent / script).is_file():
            raise SystemExit(f"{path}: background script not found: {script}")
    worker = manifest.get("background", {}).get("service_worker")
    if worker and not (path.parent / worker).is_file():
        raise SystemExit(f"{path}: service worker not found: {worker}")
    for action in manifest.get("content_scripts", []):
        for script in action.get("js", []):
            if not (path.parent / script).is_file():
                raise SystemExit(f"{path}: content script not found: {script}")
    popup = manifest.get("action", {}).get("default_popup")
    if popup and not (path.parent / popup).is_file():
        raise SystemExit(f"{path}: popup not found: {popup}")
PY

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd -- "$OUTPUT_DIR" && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

FIREFOX_STAGE="$WORK_DIR/firefox"
CHROME_STAGE="$WORK_DIR/chrome"
mkdir -p "$FIREFOX_STAGE" "$CHROME_STAGE"

copy_extension_sources() {
    local destination="$1"
    local item
    shopt -s dotglob nullglob
    for item in "$SOURCE_DIR"/*; do
        # Never package previous build output back into the next archive.
        [[ "$item" == "$SOURCE_DIR/dist" ]] && continue
        cp -a "$item" "$destination/"
    done
}

# Keep the two builds isolated so packaging never edits the working manifest.
copy_extension_sources "$FIREFOX_STAGE"
rm -f "$FIREFOX_STAGE/manifest.chromium.json"
copy_extension_sources "$CHROME_STAGE"
cp "$SOURCE_DIR/manifest.chromium.json" "$CHROME_STAGE/manifest.json"
rm -f "$CHROME_STAGE/manifest.chromium.json"

FIREFOX_ARCHIVE="$OUTPUT_DIR/hydrogen-capture-firefox.xpi"
CHROME_ARCHIVE="$OUTPUT_DIR/hydrogen-capture-chrome.zip"
rm -f "$FIREFOX_ARCHIVE" "$CHROME_ARCHIVE"

(
    cd "$FIREFOX_STAGE"
    zip -X -qr "$FIREFOX_ARCHIVE" .
)
(
    cd "$CHROME_STAGE"
    zip -X -qr "$CHROME_ARCHIVE" .
)

unzip -tq "$FIREFOX_ARCHIVE" >/dev/null
unzip -tq "$CHROME_ARCHIVE" >/dev/null

printf 'Built browser extensions:\n'
printf '  Firefox: %s\n' "$FIREFOX_ARCHIVE"
printf '  Chrome:  %s\n' "$CHROME_ARCHIVE"
printf '\nFirefox XPI is suitable for temporary installation. Normal Firefox distribution\n'
printf 'requires Mozilla signing. The Chrome ZIP is a Web Store upload package; for local\n'
printf 'testing, extract it and use chrome://extensions → Load unpacked.\n'
