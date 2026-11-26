# Filesystem adapters

This package handles filesystem discovery and extraction of file content. Keep the scanner, watcher, path policy, hashing, and format-specific extraction concerns distinct.

## Rust references

- `src/adapters/fs/scanner.rs`
- `src/adapters/fs/text_extract.rs`
- Inspect sibling files in `src/adapters/fs/` before finalizing the port list.

## Suggested modules to grow into

- Scanner: recursive traversal, exclusions, hidden files, symlinks, permissions, and error handling.
- Metadata: size, modified time, file type, and stable path representation.
- Hashing: content hashes and change detection.
- Watcher: normalized events from platform-specific implementations.
- Extraction: text, Markdown, source code, HTML, PDF, and office formats.

## Cross-platform requirements

Do not port Linux inotify directly into a shared scanner. Define a normalized event model, then implement Linux inotify, Windows ReadDirectoryChangesW, and macOS FSEvents separately. Always retain a periodic reconciliation scan because watcher events can be dropped.

## Candidate packages and resources

Use Odin's `core:os`, `core:filepath`, `core:io`, and related packages after checking their current API. For format support, evaluate mature native libraries and command-line tools individually. A dependency that works on Linux may not ship cleanly on Windows or macOS.

- [Odin core packages](https://pkg.odin-lang.org/core/)
- [Linux inotify](https://man7.org/linux/man-pages/man7/inotify.7.html)
- [Windows ReadDirectoryChangesW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesw)
- [Apple FSEvents](https://developer.apple.com/documentation/coreservices/file_system_events)

## Tests

Cover create, modify, rename, move, delete, symlink, inaccessible directory, duplicate content, unchanged file, and interrupted scan cases.
