# Domain search algorithms

This package is for deterministic search logic, not database access or model execution.

## Rust references

- `src/domain/search/tokenizer.rs`
- `src/domain/search/tfidf.rs`
- `src/domain/search/ranker.rs`
- Other modules under `src/domain/search/`

Inspect the actual Rust files before deciding whether each algorithm should remain a separate Odin source file.

## Suggested learning and implementation sequence

1. Text normalization: Unicode, case folding, punctuation, paths, and code identifiers.
2. Tokenization: preserve useful technical terms, filenames, symbols, and mathematical expressions.
3. Term statistics: document frequency, term frequency, and corpus statistics.
4. TF-IDF and/or BM25 scoring.
5. Field weighting for title, filename, path, and body.
6. Phrase matching, snippets, and deterministic tie-breaking.
7. Ranking tests using a fixed corpus.

Do not start with semantic search. A transparent lexical baseline lets you understand ranking and gives Hydrogen a useful fallback when no model is installed.

## Packages and references

Use Odin's standard string, Unicode, sorting, and math facilities where suitable. Avoid adding a third-party search engine until you have benchmarked the existing algorithms and defined the compatibility target.

- [Odin core packages](https://pkg.odin-lang.org/core/)
- [BM25 overview](https://en.wikipedia.org/wiki/Okapi_BM25)
- [TF-IDF overview](https://en.wikipedia.org/wiki/Tf%E2%80%93idf)

## Questions to answer in tests

How are empty queries handled? Are stop words removed? Are terms deduplicated? How are ties resolved? Does a query for a filename behave differently from a body-text query? Preserve current Hydrogen behaviour first, then improve it in separately measured changes.
