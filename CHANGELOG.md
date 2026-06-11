# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-06-07

### Added

- **Real EmbeddingGemma embedder in pure candle** (opt-in `gguf-embed`): a
  from-scratch forward pass of EmbeddingGemma-300m (Gemma3 encoder + dense head)
  in candle 0.10 with **no native dependencies**, loading user-supplied
  safetensors weights from a local path — never downloaded. Validated to cosine
  **0.99998** vs the official ONNX reference; 256-d Matryoshka output, asymmetric
  query/document prompts. On CodeSearchNet the vector arm goes from feature-hash
  noise (`recall@5` 0.34/0.005/0.035) to **0.95/0.99/0.885**, hybrid now beats
  BM25-only on all three slices, and the adaptive recovery gate closes — see
  [`BENCHMARK.md`](BENCHMARK.md). The **default build is unchanged**: still the
  deterministic feature-hash, still zero-model and offline.
- **Project governance & OpenSSF Best Practices artifacts**:
  [`CONTRIBUTING.md`](CONTRIBUTING.md), [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
  (Contributor Covenant 3.0), [`GOVERNANCE.md`](GOVERNANCE.md), this changelog,
  [`docs/ASSURANCE.md`](docs/ASSURANCE.md) (assurance case), and
  [`docs/best-practices-silver.md`](docs/best-practices-silver.md) (criteria
  evidence map).
- **Supply-chain / OpenSSF Scorecard hardening**: `cargo-deny` + `cargo-audit`
  jobs, a Dependabot config (Dependency-Update-Tool), a **CodeQL** workflow for
  Rust + Actions (SAST), all GitHub Actions **pinned to commit SHAs**
  (Pinned-Dependencies), and least-privilege **top-level `contents: read`** token
  permissions across every workflow (Token-Permissions), with write elevated
  per-job only where the release is created/published.
- **Fuzzing**: `cargo-fuzz` targets over the untrusted-input surface
  (`parse_source` + `chunk_file`) plus a **ClusterFuzzLite** setup that runs them
  on PRs (Scorecard Fuzzing). The `fuzz/` crate is isolated from the main
  workspace.
- **Registry publishing**: `release.yml` now publishes the crates to **crates.io**
  (`cargo publish`, indexer first then bin) and the npx wrapper to **npm**
  (Scorecard Packaging). Adds the crate metadata crates.io requires
  (`description`/`keywords`/`categories`/`repository`).
- **Branch protection** on `main`: PRs + strict status checks (CI, CodeQL, deny,
  audit, offline-isolation) + linear history + no force-push, enforced for admins.

### Changed

- **`chunks_vec` width is parametrized by the active embedder's dimension**
  (`open_db_with(path, dim)`), decoupling the vector-table DDL from the
  `EMBED_DIM = 384` feature-hash constant so an opt-in model with a different
  dimension (e.g. EmbeddingGemma 256/768) stores correctly. The default path
  (`open_db`) stays byte-identical; the existing refuse-to-mix guard rejects an
  index built with a different embedder id/dim.

## [0.2.0-rc.1] - 2026-06-06

### Added

- **Adaptive query-shape fusion weighting** (opt-in, default off): biases the
  BM25/vector fusion by query shape (`search_code(adaptive: true)`).
- **CodeSearchNet benchmark harness**: offline `recall@5`/`MRR` across four arms
  (BM25-only / vector-only / hybrid / hybrid+MMR) on the standard Python/Go/
  TypeScript `{NL query → code}` slices, env-pointed and never vendored.
- **Multi-repo schema**: each repo keeps its own SQLite index keyed on a
  composite `PRIMARY KEY(repo_id, path)`, with a sidecar JSON registry tracking
  the path→index map and versioned, backward-compatible migrations.
- **SLSA Build L3 signed provenance** on every release artifact, via `cargo-dist`
  native GitHub attestation (`build-local-artifacts` phase).
- **`SECURITY.md` threat model** + an **OpenSSF Scorecard** workflow and README
  supply-chain badges.
- **Opt-in real embedder scaffolding** (`gguf-embed`): an initial candle BERT
  backend (later superseded by the EmbeddingGemma path above), behind a feature
  flag with the default build kept zero-model.
- **Walker hardening**: skip generated/minified assets by maximum line length to
  avoid index bloat; per-language chunk-cap validation for TypeScript and Python.

## [0.1.0] - 2026-06-05

Initial release of **apohara-codesearch** — an offline hybrid code-search MCP
server: one Rust binary, no model, no database.

### Added

- **MCP stdio server** exposing two tools — `search_code` (hybrid BM25 + vector
  search, lazy first-index) and `reindex` (incremental, blake3 content-hash
  deltas) — over plain JSON-RPC.
- **Structural extraction** (tree-sitter) for **Rust, TypeScript, Python, and
  Go**: per-symbol chunks with signatures + file imports/exports; any other
  language is indexed as overlapping text windows.
- **Hybrid retrieval**: BM25 (SQLite FTS5) + a deterministic blake3 feature-hash
  vector (sqlite-vec), merged with Reciprocal Rank Fusion, MMR-diversified, with
  an optional structural import boost. Same input ⇒ same vector ⇒ stable
  `recall@k`/`MRR`.
- **`watch` subcommand**: keeps the index current as files change (a plain CLI
  loop, not a plugin hook).
- **npm wrapper** (`@apohara/codesearch-mcp`): downloads the matching prebuilt
  binary from the GitHub Release and runs the MCP server via `npx`.
- **Honest benchmark** (`BENCHMARK.md`): synthetic in-CI corpus plus a one-off
  external real-OSS comparison, with ≥30% committed known-miss queries.
- **Dual license**: MIT OR Apache-2.0.

[Unreleased]: https://github.com/SuarezPM/apohara-codesearch/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/SuarezPM/apohara-codesearch/compare/v0.2.0-rc.1...v0.2.0
[0.2.0-rc.1]: https://github.com/SuarezPM/apohara-codesearch/compare/v0.1.0...v0.2.0-rc.1
[0.1.0]: https://github.com/SuarezPM/apohara-codesearch/releases/tag/v0.1.0

## [0.3.0] - 2026-06-11

### Added

- **5 new tree-sitter grammars** (Bash, Java, C, Ruby, C++) for the structural
  extractor. The engine now supports **9 languages** for symbol-level
  indexing (function definitions, type declarations, imports/exports):
  Rust, TypeScript, Python, Go (the historical set) + Bash, Java, C, Ruby,
  C++. Each new grammar ships its own parser, import extractor, fuzz
  target, and a checked-in fixture under `tests/fixtures/`. **Kotlin is
  deferred to v0.4.0** per the plan's barred-entry rule (R-1.1):
  tree-sitter-kotlin has no 0.23.x line on crates.io, which would break the
  workspace's tree-sitter version pin policy.
- **Corpus freezes for the v0.3.0 measurement** (`.omc/plans/.../corpus_freeze.rs`):
  - `tests/fixtures/bench-corpus-frozen-A/`: 22-file copy of
    `examples/bench-corpus/` at the v0.2.0 commit, content-hash pinned.
  - `tests/fixtures/bench-corpus-frozen-B/queries.json`: 10-query golden-test
    subset for the F3 default-flip measurement.
  - The guard test `crates/apohara-codesearch/tests/corpus_freeze.rs`
    fails on any drift; refreezing requires a `chore(bench): refreeze
    corpus X` commit.
- **OpenSSF Scorecard audit** (`.omc/plans/apohara-codesearch-scorecard-audit.md`):
  measured aggregate **7.0/10** (not 5.8 — that number in `CLAUDE.md` was
  stale). 9 of 18 checks at 10/10, 4 at 0-4. The QW-2 fix pins
  `cargo-audit` to the `Cargo.lock` version (`ee8b06a`). QW-1 (Maintained)
  is a structural repo-age penalty that resolves itself after 90 days.
  QW-3 re-score showed 0 immediate delta (scorecard needs 24-48h to
  re-index); expected +2 once indexed.
- **F3 BENCHMARK baseline** (BENCHMARK.md v0.3.0 section): the v0.2.0
  hybrid-search baseline on the frozen corpus A is BM25
  recall@5=0.542/recall@10=0.625/MRR=0.326, vector 0.083/0.083/0.063,
  hybrid 0.458/0.542/0.285, with 9/24 queries where hybrid < best
  single mode (38%). The bench surface cannot measure the
  proposed-flip variants directly (see "Changed" below for why the flips
  are deferred).

### Changed

- **No default flips this cycle.** Per the v0.3.0 plan
  (`.omc/plans/apohara-codesearch-3frentes.md` §6), the proposed
  `adaptive=true` / `diversify=true` default flips require a
  data-driven positive-lift measurement that the bench-search harness
  cannot produce (both opt-ins live in the server-side `search_code`
  wrapper, not in the indexer-level `rrf_fuse`). F3-FLIP-CHECK
  therefore has no data to apply the split criteria, and Pablo
  chose to defer both flips to v0.4.0 with the appropriate plumbing
  to measure them server-side. **The v0.3.0 release is therefore
  structural-extraction-focused, not ranking-focused.** Rollback
  path for the flips is documented in the plan §10 and remains
  valid for the v0.4.0 measurement.
- **`legacy.rb` fixture renamed to `legacy.foo`** in
  `examples/demo-repo/`. Reason: the v0.3.0 grammar expansion
  means `.rb` is now a parsed language; the ac4 integration test
  needed an extension no grammar recognizes. The file's content is
  unchanged.
- **`test_detect_language_c`** updated to reflect that C++ extensions
  (`.cpp`/`.hpp`/`.cc`/`.cxx`/`.hxx`/`.hh`) now map to `Language::Cpp`
  instead of returning `None`. The C vs C++ split follows the
  tree-sitter convention (one grammar per major).
- **Module symbol kind added to `SymbolKind` enum** (Ruby `module`
  declaration support).
- **Workspace tree-sitter dep set expanded**: `tree-sitter-bash`,
  `tree-sitter-java`, `tree-sitter-c`, `tree-sitter-ruby`,
  `tree-sitter-cpp` (all at 0.23.x to match the existing pin).

### Notes

- **Binary size on linux-x64: +7.99 MB (+62.58%) vs v0.2.0.** Each new
  tree-sitter grammar contributes ~0.5-3.5 MB to the statically-linked
  binary (the C parser-table C code is the dominant cost; Java
  surprised as the smallest at +0.43 MB, Ruby at +2.05 MB, C++ at
  +3.45 MB). Pablo approved "all 6 grammars default" at the
  size-budget gate (the cumulative projection was revised from +60%
  to +62.58% as the actual measurements came in). The
  v0.3.0 plan's C++/SACRED resolution still applies: the
  windows-msvc artifact has a +20% budget; if the windows-msvc
  build exceeds it, C++ goes per-target `default = []` and is
  opt-in via `cargo build --features cpp`. This must be verified
  at the F3-RELEASE / CI step.
- **OpenSSF Scorecard**: 7.0/10 baseline measured. The 3 quick wins
  approved by Pablo (pin cargo-audit) are committed. No further
  Scorecard work in this release; the audit doc remains the source
  of truth for follow-ups.
- **Kotlin deferred to v0.4.0** — see "Added" notes.

[Unreleased]: https://github.com/SuarezPM/apohara-codesearch/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/SuarezPM/apohara-codesearch/compare/v0.2.0...v0.3.0
