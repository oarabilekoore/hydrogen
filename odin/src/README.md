# Odin source layout

This directory mirrors the high-level shape of the Rust `src/` tree. Odin treats a directory as a package, so the package declarations in each directory must agree.

## Entry point

- `main.odin`: process startup, command dispatch, initialization, shutdown, and top-level error reporting.
- `config.odin`: configuration loading and validation, corresponding to the Rust root-level configuration module.

## Subsystems

- `domain/`: core data types and search algorithms. Keep this independent of SQLite, the operating system, and external services.
- `application/`: use-case orchestration. It coordinates domain logic and adapter interfaces.
- `adapters/`: concrete integrations with filesystems, databases, and ML runtimes.
- `cli/`: command-line parsing and command handlers.

## Odin package rule

Each directory is a separate package. Files in the same directory share one package name; nested directories are separate packages and do not automatically import their parent. Start with explicit, small package boundaries rather than creating a large package with hidden cross-module dependencies.

## Implementation exercise

Trace one command from `main.rs` in Rust through configuration, application service, adapter, and output. Draw the call graph and data flow before implementing it in Odin. This is a useful way to learn package imports and procedure contracts without translating blindly.

Resources: [Odin overview](https://odin-lang.org/docs/overview/), [package reference](https://pkg.odin-lang.org/), [core library](https://pkg.odin-lang.org/core/).
