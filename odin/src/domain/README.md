# Domain layer

The domain layer describes Hydrogen's concepts and rules without knowing how they are stored or which operating system is running.

## Responsibilities

- Document and source identity.
- File metadata and content-derived metadata.
- Search query and result models.
- Passage, page, image, and embedding concepts.
- Validation and domain-level invariants.

## Rust references

Start with `src/domain/document.rs` and inspect the rest of `src/domain/`. Record which types are persisted, which are transient, and which are API-facing. Do not assume that similarly named Rust and Odin structures need identical memory layouts.

## What to implement

1. Define stable identifiers and document/source structures.
2. Separate a filesystem path from a document's logical identity.
3. Model optional metadata explicitly.
4. Define clear ownership for strings, byte buffers, and collections.
5. Add equality and validation tests for important invariants.

## Useful Odin concepts

Study structs, tagged unions, enums, slices, dynamic arrays, maps, procedures, and allocators. Prefer value types for small fixed metadata and explicit ownership for larger buffers.

Resources: [Odin overview](https://odin-lang.org/docs/overview/), [Odin package API](https://pkg.odin-lang.org/).
