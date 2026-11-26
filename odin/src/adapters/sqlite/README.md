# SQLite adapter

This package owns database connections, statements, transactions, schema migrations, and repository implementations. It is an infrastructure boundary, not the place for search ranking or indexing policy.

## Rust references

- `src/adapters/sqlite/repo.rs`
- `src/adapters/sqlite/knowledge.rs`
- Inspect the current schema and migration code before changing any table or index.

## First objective: compatibility

Preserve the current schema version and open a copied Hydrogen database read-only first. Then implement schema inspection and integrity checks. Do not let an early Odin experiment mutate the only copy of a user's data.

## Suggested implementation order

1. SQLite open/close and error reporting.
2. Prepared statement lifecycle and typed binding/column access.
3. Transactions and rollback behaviour.
4. Schema version inspection and migration runner.
5. Document and file repositories.
6. Search, vector, conversation, and knowledge repositories.
7. Busy timeout, WAL policy, concurrency, and backup behaviour.

## Candidate dependency

The SQLite C API is a strong initial option because it is stable and widely deployed. Bind only the functions needed, check every return code, and make statement/connection ownership explicit. Consider whether to link SQLite statically or dynamically based on packaging goals.

Resources: [SQLite C API](https://www.sqlite.org/c3ref/intro.html), [SQLite transactions](https://www.sqlite.org/lang_transaction.html), [SQLite WAL](https://www.sqlite.org/wal.html), [Odin foreign system](https://odin-lang.org/docs/overview/#foreign-system).
