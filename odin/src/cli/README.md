# Command-line interface

This package translates command-line input into application use cases and presents human-readable output. Keep command parsing separate from the services that perform the work.

## Rust references

Inspect all files under `src/cli/`, especially the command dispatch and server command. The existing CLI code is large, so document commands and flags before porting handlers.

## Suggested command groups

- `version`, `config`, and `doctor` for diagnostics.
- `index` or `sync` for filesystem indexing.
- `search` for lexical and semantic queries.
- `serve` for the local HTTP API.
- Model management and database inspection commands.

## Implementation guidance

Start with a minimal command parser and a command-to-application-service map. Preserve exit codes, stdout/stderr behaviour, flag names, defaults, and machine-readable output where scripts depend on them.

Resources: [Odin core flag package](https://pkg.odin-lang.org/core/flag/), [Odin core fmt package](https://pkg.odin-lang.org/core/fmt/).
