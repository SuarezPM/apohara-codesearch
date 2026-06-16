---
name: freeze-benchmark-corpus-and-guard
description: Workflow command scaffold for freeze-benchmark-corpus-and-guard in apohara-codesearch.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /freeze-benchmark-corpus-and-guard

Use this workflow when working on **freeze-benchmark-corpus-and-guard** in `apohara-codesearch`.

## Goal

Freezes a benchmark corpus for reproducible benchmarking and adds a guard test to pin content hashes.

## Common Files

- `tests/fixtures/bench-corpus-frozen-*/`
- `crates/apohara-codesearch/tests/corpus_freeze.rs`
- `Cargo.toml`
- `Cargo.lock`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Copy or update files in tests/fixtures/bench-corpus-frozen-*/.
- Add or update guard test to check content hashes (crates/apohara-codesearch/tests/corpus_freeze.rs).
- Update or add relevant dev-dependencies (e.g., sha2 in Cargo.toml, Cargo.lock).
- Document the freeze and hashes in commit message and/or BENCHMARK.md.

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.