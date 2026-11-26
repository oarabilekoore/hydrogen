# Odin source layout

This directory mirrors the high-level shape of the Rust `src/` tree. Odin treats a directory as a package, so the package declarations in each directory must agree.

## Entry point

- `main.odin`: process startup, global argument parsing, command dispatch, initialization, shutdown, and top-level error reporting.
- `version.odin`: the Odin rewrite's version constant. Keep it aligned with the root `Cargo.toml` until versioning is deliberately separated.
- `config.odin`: configuration loading and validation, corresponding to the Rust root-level configuration module.

## Current CLI slice

The initial executable behaviour is intentionally small:

- `hydrogen version`
- `hydrogen --version` or `hydrogen -V`
- `hydrogen --help` or `hydrogen -h`

Build from the repository root with `odin build odin/src -out:hydrogen`, then run `./hydrogen version`.

### Argument parsing and dispatch

Global options are parsed with Odin's standard `core:flags` package using UNIX-style arguments. The `Global_Args` struct defines the supported global flags, while its `overflow` field collects positional arguments. The first positional argument is the command name.

This separation is deliberate:

1. `core:flags` parses options and reports malformed or unknown flags.
2. The command dispatcher selects the command handler.
3. Each command can later define its own argument struct and call `core:flags` for command-specific options.

Do not turn the dispatcher into a second argument parser. It should only decide which command to run.

The standard parser reserves `-h` and `-help` for help requests. The global version option is explicitly defined in the argument struct, including a hidden `-V` alias.

## Subsystems

- `domain/`: core data types and search algorithms. Keep this independent of SQLite, the operating system, and external services.
- `application/`: use-case orchestration. It coordinates domain logic and adapter interfaces.
- `adapters/`: concrete integrations with filesystems, databases, and ML runtimes.
- `cli/`: command handlers and command-specific argument models.

## Odin package rule

Each directory is a separate package. Files in the same directory share one package name; nested directories are separate packages and do not automatically import their parent. Start with explicit, small package boundaries rather than creating a large package with hidden cross-module dependencies.

## Implementation exercise

Trace one command from `main.rs` in Rust through configuration, application service, adapter, and output. Draw the call graph and data flow before implementing it in Odin. This is a useful way to learn package imports and procedure contracts without translating blindly.

Resources: [Odin overview](https://odin-lang.org/docs/overview/), [package reference](https://pkg.odin-lang.org/), [core library](https://pkg.odin-lang.org/core/), [core:flags](https://pkg.odin-lang.org/core/flags/).
