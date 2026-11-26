# Application services

Application services implement user-visible use cases by coordinating domain logic and adapter interfaces. They should not contain SQL statements, platform-specific system calls, or model-runtime details.

## Rust references

- `src/application/sync_service.rs`
- `src/application/query_service.rs`
- `src/application/ocr_service.rs`
- `src/application/watcher_service.rs`

The Rust sync and CLI modules are large. Resist the temptation to recreate them as single giant Odin procedures. Split orchestration into small steps with explicit inputs, outputs, and failure handling.

## Responsibilities

- Sync/index orchestration and job lifecycle.
- Query orchestration and result assembly.
- OCR workflow coordination.
- Watcher event handling and reconciliation.
- Retry policy, cancellation, progress, and metrics.

## Suggested design

Define the application workflow in terms of operations required from adapters. Keep adapter contracts small and testable. A sync operation should be safe to retry and should distinguish discovered, changed, unchanged, failed, and deleted files.

## Useful Odin concepts

Learn procedure types, explicit error/result conventions, dynamic arrays, allocator ownership, threading, mutexes, and channels before implementing concurrent workers. Start single-threaded; add concurrency only after correctness tests exist.

Resources: [Odin overview](https://odin-lang.org/docs/overview/), [core sync packages](https://pkg.odin-lang.org/core/sync/), [core thread packages](https://pkg.odin-lang.org/core/thread/).
