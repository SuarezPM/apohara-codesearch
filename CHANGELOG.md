# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/SuarezPM/apohara-codesearch/compare/v0.2.0-rc.1...HEAD
[0.2.0-rc.1]: https://github.com/SuarezPM/apohara-codesearch/compare/v0.1.0...v0.2.0-rc.1
[0.1.0]: https://github.com/SuarezPM/apohara-codesearch/releases/tag/v0.1.0
