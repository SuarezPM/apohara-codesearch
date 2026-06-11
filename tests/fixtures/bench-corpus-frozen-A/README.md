# bench-corpus

A small, checked-in, multi-language synthetic repository used by the
`bench-search` retrieval benchmark (see top-level `BENCHMARK.md`).

It is deliberately coherent and distinguishable: each file owns a distinct topic
and vocabulary so a labeled query has exactly one obviously-relevant target. The
languages cover the indexer's full surface:

- **Rust** (`src/billing.rs`, `src/geometry.rs`, `src/text_search.rs`,
  `src/retry.rs`, `src/checksum.rs`, `src/temperature.rs`, `src/queue.rs`) —
  functions, methods, and type declarations with structural imports/exports.
- **TypeScript** (`src/validation.ts`, `src/strings.ts`, `src/cache.ts`,
  `src/auth.ts`, `src/color.ts`) — classes, interfaces, and free functions.
- **Python** (`src/pricing.py`, `src/stats.py`, `src/sorting.py`) — functions
  and a class.
- **Go** (`src/ratelimit.go`, `src/graph.go`, `src/config.go`) — structs,
  interfaces, methods, and functions.
- **Unparsed** (`docs/architecture.md`, `CHANGELOG.txt`) — windowed text with no
  symbol rows, exercising text-only retrieval over prose.

The corpus is authored by the tool's author, so the *absolute* benchmark numbers
deserve skepticism — the methodology and the committed known-miss cases are the
credibility anchor, not the corpus. The labels and the relevance rule live in
the harness (`crates/apohara-codesearch/src/bin/bench-search.rs`).

Reproduce: `cargo run --release --bin bench-search`.
