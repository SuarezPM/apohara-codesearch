---
name: add-new-tree-sitter-grammar
description: Workflow command scaffold for add-new-tree-sitter-grammar in apohara-codesearch.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /add-new-tree-sitter-grammar

Use this workflow when working on **add-new-tree-sitter-grammar** in `apohara-codesearch`.

## Goal

Adds support for a new programming language by integrating a tree-sitter grammar, updating the parser, adding fixtures, tests, and fuzz targets.

## Common Files

- `Cargo.toml`
- `Cargo.lock`
- `crates/apohara-indexer/Cargo.toml`
- `crates/apohara-indexer/src/parser.rs`
- `crates/apohara-indexer/src/incremental.rs`
- `crates/apohara-indexer/tests/fixtures/fixture.*`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Add new tree-sitter-<lang> dependency to workspace (Cargo.toml, Cargo.lock, crates/apohara-indexer/Cargo.toml).
- Update parser logic to support new Language variant (crates/apohara-indexer/src/parser.rs, incremental.rs).
- Add language detection and extraction logic (parser.rs, incremental.rs).
- Add new test fixtures for the language (crates/apohara-indexer/tests/fixtures/fixture.<ext>, imports.<ext>).
- Write unit tests for detection, extraction, and import logic (crates/apohara-indexer/src/parser.rs, tests/).

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.