<div align="center">

# apohara-codesearch

**Hybrid code search for your coding agent — _one offline binary, no model, no database, 9 languages._**

[![CI](https://img.shields.io/github/actions/workflow/status/SuarezPM/apohara-codesearch/ci.yml?style=for-the-badge&label=CI)](https://github.com/SuarezPM/apohara-codesearch/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=for-the-badge)](#-license)
[![Rust](https://img.shields.io/badge/rust-stable-orange?style=for-the-badge&logo=rust)](https://www.rust-lang.org)
[![crates.io](https://img.shields.io/crates/v/apohara-codesearch?style=for-the-badge&logo=rust&label=crates.io)](https://crates.io/crates/apohara-codesearch)
[![npm](https://img.shields.io/npm/v/@apohara/codesearch-mcp?style=for-the-badge&label=npm&color=purple)](https://www.npmjs.com/package/@apohara/codesearch-mcp)
[![MCP](https://img.shields.io/badge/MCP-stdio%20server-success?style=for-the-badge)](https://modelcontextprotocol.io)

[![OpenSSF Scorecard](https://img.shields.io/ossf-scorecard/github.com/SuarezPM/apohara-codesearch?style=for-the-badge&label=Scorecard)](https://scorecard.dev/viewer/?uri=github.com/SuarezPM/apohara-codesearch)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/13118/badge)](https://www.bestpractices.dev/projects/13118)

**[Quick Start](#-quick-start)** · **[Features](#-features)** · **[Languages](#-supported-languages)** · **[Where it fits](#-where-it-fits)** · **[How it works](#-how-it-works--honesty)**

**Latest release: [v0.3.0](https://github.com/SuarezPM/apohara-codesearch/releases/tag/v0.3.0) — 2026-06-11** — published to [crates.io](https://crates.io/crates/apohara-codesearch) and [npm](https://www.npmjs.com/package/@apohara/codesearch-mcp); SLSA Build L3 provenance on every artifact. **9 languages** for structural extraction (Bash, C, C++, Go, Java, Python, Ruby, Rust, TypeScript) — up from 4 in v0.2.0 — with corpus freezes and an audited OpenSSF Scorecard of **7.0/10**.

A single Rust binary that runs as a [Model Context Protocol](https://modelcontextprotocol.io) server, giving a coding agent fast, **fully-offline** hybrid search over any local repository — no embedding model to download, no external vector or graph database. It installs in seconds, runs air-gapped in a few megabytes of RAM, and keeps its entire state in **one SQLite file**.

</div>

---

```console
# Your agent calls the search_code MCP tool:
search_code(path=".", query="where does the runtime block on a future?")

# → top hit — a chunk WITH its structure, not just a line number:
{
  "file": "src/runtime/handle.rs",
  "start_line": 241, "end_line": 341,
  "kind": "method",
  "signature": "block_on<F: Future>(&self, future: F) -> F::Output",
  "snippet": "/// Runs a future to completion on this Handle's associated Runtime...",
  "imports": [{ "source": "crate::runtime::task::JoinHandle", "line": 17 }],
  "exports": []
}
```

> Structure (signatures, imports/exports, and `struct`/`enum`/`class`/`interface`/`module`/`type` symbols) is extracted for **9 languages**: Rust, TypeScript, Python, Go, **Bash, Java, C, Ruby, C++** (new in v0.3.0). Any other language is still searchable — indexed as overlapping text windows.

---

## 💡 Concept

> [!NOTE]
> **A hash, not a model.** The dominant code-intelligence tools are heavy: Node plus native bindings, a C/C++ toolchain for some grammars, an embedded graph or vector database, and a learned embedding model that downloads on first run. Their strength is deep understanding; their cost is that they are anything but lightweight.

`apohara-codesearch` takes the other side of that trade. The embedding is a deterministic [blake3](https://github.com/BLAKE3-team/BLAKE3) feature-hash, **not** a learned model — so there is nothing to download, nothing to serve, and the same input always produces the same vector. That makes semantic recall weaker than a model-based tool; we compensate with **hybrid retrieval** (lexical + vector, fused) rather than pretending the hash is semantic. It is a Claude Code MCP plugin, and works with any MCP client.

> [!TIP]
> **The v0.3.0 trade is breadth, not depth.** Five new languages added in one release (Bash, Java, C, Ruby, C++); Kotlin deferred to v0.4.0 (see [open-questions.md](.omc/plans/open-questions.md) — tree-sitter-kotlin's version line on crates.io does not match the workspace's pin policy). The default install stays offline and zero-deps; you trade a one-time +63% binary-size delta on linux-x64 for a four-fold increase in languages your agent can structurally understand.

---

## ✨ Features

| | |
|---|---|
| 🔌 **MCP stdio server** | Two tools — `search_code` (hybrid) and `reindex` (incremental) — over plain JSON-RPC. Works with Claude Code or any MCP client. |
| 🦀 **One static binary** | No Node, no native bindings, no toolchain, no service. `cargo install` or `npx`, then run. The only state is one SQLite file. |
| 🌳 **9 languages for structural extraction** *(v0.3.0)* | Rust, TypeScript, Python, Go, **Bash, Java, C, Ruby, C++** — per-symbol chunks with signatures + imports/exports. |
| 🧠 **Hybrid ranking** | BM25 (SQLite FTS5) + a feature-hash vector (sqlite-vec), merged with Reciprocal Rank Fusion, then MMR-diversified. Optional **adaptive weighting** biases the fusion by query shape (opt-in, off by default). |
| 📴 **Offline & air-gapped** | Zero network at runtime AND at build. No model fetch, no telemetry, no API keys. |
| 🪶 **Near-zero footprint** | ~22 MB resident memory indexing a 224k-LOC repo — flat with repo size (memory-bounded pipeline). |
| 🗂️ **Multi-repo aware** | Each repo keeps its own SQLite index (`PRIMARY KEY(repo_id, path)`); a sidecar JSON registry tracks the path→index map, with versioned, backward-compatible migrations. |
| ⚡ **Incremental + watch** | `reindex` does blake3 content-hash deltas; the `watch` subcommand keeps the index current as files change (a plain CLI loop, **not** a plugin hook). |
| 🔁 **Deterministic** | Same input ⇒ same vector ⇒ byte-stable `recall@k`/`MRR`. Re-indexing is stable. |
| 🛡️ **Hardened supply chain** *(v0.3.0)* | OpenSSF Scorecard **7.0/10** (audited), Best Practices **Silver** badge #13118, **SLSA Build L3** provenance, `cargo-deny` + `cargo-audit` + Dependabot, all Actions pinned to commit SHAs, branch protection on `main`. |

---

## 🌐 Supported Languages

| Language | Symbols | Imports | Exports | Version |
|---|---|---|---|---|
| **Rust** | functions, structs, enums, traits, types | `use` + `mod` | `pub` items | v0.1.0 |
| **TypeScript** | functions, classes, interfaces, types, methods | `import` (named/default/namespace/side-effect/require) | `export` (named/default/re-export) | v0.1.0 |
| **Python** | functions, classes, methods | `import` + `from ... import` | (no syntactic export) | v0.1.0 |
| **Go** | functions, methods, structs, interfaces, types | `import` blocks | (no syntactic export) | v0.1.0 |
| **Bash** | functions | `source` + `.` (Require) | `export` builtin | **v0.3.0** |
| **Java** | classes, interfaces, enums, records, methods, constructors | `import` (scoped + static) | (visibility via type-symbol pass) | **v0.3.0** |
| **C** | functions | `#include` (system + local) | (no syntactic export) | **v0.3.0** |
| **Ruby** | classes, modules, methods, singleton methods | `require` + `require_relative` | (no syntactic export) | **v0.3.0** |
| **C++** | free functions, member functions, classes, structs | `#include` (system + local) | (no syntactic export) | **v0.3.0** |
| **Kotlin** | *(deferred to v0.4.0 — tree-sitter-kotlin's version line on crates.io does not match the workspace's 0.23.x pin)* | | | v0.4.0 |

Any other file (`.txt`, `.md`, `.json`, custom extensions) is still indexed as overlapping text windows — the **works on any repo** promise is preserved.

---

## 🚀 Quick Start

Register it with your MCP client. For **Claude Code**, add to `.mcp.json`:

```json
{ "mcpServers": { "codesearch": { "command": "npx", "args": ["-y", "@apohara/codesearch-mcp"] } } }
```

The `npx` wrapper downloads the matching prebuilt binary for your platform on first run. That is the whole install — no model, no database, no daemon.

<details>
<summary><b>Other acquisition paths</b> — build from source, run directly, keep the index live</summary>

```bash
# Install from crates.io:
cargo install apohara-codesearch

# Or build + install from a checkout (lowest-trust path):
cargo install --path crates/apohara-codesearch

# Run the binary directamente as a stdio MCP server:
apohara-codesearch serve

# Keep the index current as files change (plain CLI loop, NOT a Claude Code hook):
apohara-codesearch watch <path>
```

Prebuilt, per-OS binaries are also published on [Releases](https://github.com/SuarezPM/apohara-codesearch/releases) (built by `cargo-dist`). It installs as a Claude Code plugin via the `apohara` marketplace too.

> [!WARNING]
> Downloading a prebuilt binary is itself a supply-chain surface. Verify the checksum from the Release, or prefer `cargo install` and build from source.

Every release artifact carries a signed **SLSA Build L3** provenance attestation, generated by `cargo-dist` at build time. Verify it with the GitHub CLI:

```sh
gh attestation verify <artifact> --repo SuarezPM/apohara-codesearch
```

This proves the binary was built by this repo's release workflow on GitHub-hosted runners — a stronger guarantee than a plain checksum, which only proves the file matches what the Release page claims.

</details>

### Tools

| Tool | What it does |
|---|---|
| `search_code` | Hybrid BM25 + vector search over a repo path. Lazily indexes on first call. Returns the top-k hits with structural context. Optional knobs: `bm25_weight`/`vector_weight` (explicit fusion weights), `adaptive` (query-shape weighting, off by default), `diversify` (MMR), `boost_imports`. |
| `reindex` | Re-index a repo. Incremental by default (blake3 content-hash deltas); `force: true` rebuilds from scratch. |

---

## 🧭 Where it fits

Lighter than the graph tools, structure-aware where `ripgrep` is text-only. It does **not** match a model-based tool on conceptual recall, and it does **not** build a call graph — those are deliberately out of scope.

| | apohara-codesearch | graph / embedding tools | ripgrep |
|---|---|---|---|
| **Runtime dependencies** | one static binary | Node + native bindings + toolchain | one binary |
| **Model download** | none | hundreds of MB | none |
| **External DB / service** | none | embedded graph / vector DB | none |
| **Offline / air-gapped** | ✓ | usually requires a fetch | ✓ |
| **Structural context** | signatures + imports/exports (9 langs) | call graphs, deep | text only |
| **Languages w/ symbols** *(v0.3.0)* | **9** (Bash, C, C++, Go, Java, Python, Ruby, Rust, TS) | usually broader, deeper | none |
| **Ranking** | hybrid BM25 + vector (RRF) | learned embeddings | exact / regex |
| **OpenSSF Scorecard** *(v0.3.0)* | **7.0/10** | varies | n/a |

---

## 🔬 How it works / honesty

1. **Walk + chunk.** A `.gitignore`-aware walk splits each file into per-symbol chunks (with the symbol's signature attached) plus bounded module-remainder and window chunks, so a giant file never collapses into one diluted chunk. Each language's `tree-sitter` parser (9 of them as of v0.3.0) drives the symbol extraction.
2. **Index.** Each chunk gets a BM25 lexical row (SQLite FTS5) and a feature-hash vector row (sqlite-vec), keyed on a shared row id. Both sides share one identifier tokenizer, so `parseString` and `parse_string` match each other.
3. **Search.** A query runs through both BM25 and vector k-NN; the two ranked lists are merged with [Reciprocal Rank Fusion](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf), diversified with MMR, then the survivors are hydrated with their structural context.
4. **Stay current.** Re-indexing hashes each file and reprocesses only what changed, in a single transaction that keeps the three tables consistent.

### What's new in v0.3.0 (at a glance)

- **9 languages for structural extraction** — up from 4 in v0.2.0. Each new grammar ships its own parser, import extractor, fuzz target, and a checked-in fixture under `tests/fixtures/`. One opt-in per language: `apohara-codesearch --language java` (or whatever the dispatch decides from the file extension).
- **Corpus freezes for the v0.3.0 measurement** — two content-hash-pinned copies of the bench corpus live at `tests/fixtures/bench-corpus-frozen-A/` (22 files, byte-identical to `examples/bench-corpus/` at v0.2.0) and `tests/fixtures/bench-corpus-frozen-B/queries.json` (10-query golden-test subset). A guard test at `crates/apohara-codesearch/tests/corpus_freeze.rs` fails on any drift. Refreezing requires a `chore(bench): refreeze corpus X` commit and is auditable.
- **OpenSSF Scorecard audit** (`.omc/plans/apohara-codesearch-scorecard-audit.md`) — the actual measured score is **7.0/10** (not 5.8 as `CLAUDE.md` previously stated; that was stale). 9 of 18 checks at 10/10, 4 at 0-4. The QW-2 fix in this release pins `cargo-audit` to the `Cargo.lock` version (`ee8b06a`); QW-1 (Maintained) is a structural repo-age penalty that resolves itself after 90 days; QW-3 re-score showed 0 immediate delta (scorecard needs 24-48h to re-index), expected +2 once indexed.
- **F3 BENCHMARK baseline** (see the v0.3.0 section in [BENCHMARK.md](BENCHMARK.md)) — the v0.2.0 hybrid-search baseline on the frozen corpus A: BM25 recall@5=0.542/recall@10=0.625/MRR=0.326, vector 0.083/0.083/0.063, hybrid 0.458/0.542/0.285, with 9/24 queries where hybrid < best single mode (38%). The bench surface cannot measure the proposed-flip variants directly (adaptive/diversify live in the server-side `search_code` wrapper, not in the indexer-level `rrf_fuse`); this is the v0.4.0 follow-up.
- **No default flips this cycle** — both proposed flips (`adaptive=true`, `diversify=true`) require a data-driven positive-lift measurement that the bench-search harness cannot produce. They are deferred to v0.4.0 with the appropriate plumbing to measure them server-side. v0.3.0 is therefore **structural-extraction-focused, not ranking-focused**.
- **OQ-1 v0.8 (Identity Gate) closed** in `.omc/plans/open-questions.md` — opted for **A2 (candle-pure, no ort/ONNX)** to preserve the wedge. The opt-in embedder path remains available behind `--features gguf-embed` but a real default-flipped embedder requires a code-trained model (OQ-3 v0.5 follow-up; v0.4.0).

### Footprint at scale

Measured with the default feature-hash embedder on a Ryzen 5 3600 / 48 GB box, driven over the stdio MCP tools:

| Repo | LOC | Cold index | Peak RSS | Warm query | Index on disk |
|---|---|---|---|---|---|
| [tokio](https://github.com/tokio-rs/tokio) | 174k Rust | ~10 s | ~22 MB | ~18 ms | 39 MB |
| [hugo](https://github.com/gohugoio/hugo) | 224k Go | ~26 s | ~24 MB | ~22 ms | 54 MB |

Peak resident memory is **flat across repo size** — no OOM, no external process. One SQLite file is the only state.

> [!WARNING]
> **The vector is a robustness layer, not a semantic engine.** Because the embedding is a feature-hash, a conceptual query that shares no tokens with the target will not surface it — and on a clean corpus where lexical search already wins, fusion can be a slight net negative. [BENCHMARK.md](BENCHMARK.md) **publishes this** (synthetic corpus + a one-off external comparison on real OSS, with ≥30% committed known-miss queries) rather than hiding it. Deep structural context (callers/callees, call graphs) is out of scope by design. A real local embedding model is an **opt-in, user-supplied** build feature — never downloaded — so the default install stays zero-dependency. **That lever now exists and is measured:** the `gguf-embed` feature runs **EmbeddingGemma-300m in pure candle** (no native deps) on user-supplied weights, and on CodeSearchNet it flips this picture entirely — the vector arm beats BM25-only and fusion stops being a tax (see [BENCHMARK.md](BENCHMARK.md), v0.8 section). The default build is unchanged: still the deterministic feature-hash, still zero-model.

### Binary-size note (v0.3.0)

Five new `tree-sitter` grammars (Bash, Java, C, Ruby, C++) added **+7.99 MB / +62.58%** to the statically-linked binary on linux-x64. Each grammar contributes ~0.5-3.5 MB (the C parser-table code is the dominant cost; Java surprised as the smallest at +0.43 MB, Ruby at +2.05 MB, C++ at +3.45 MB). This is the cost of more languages; the per-language delta shrinks for the next grammar added. The v0.3.0 plan's C++/SACRED resolution still applies: the windows-msvc artifact has a +20% budget; if the windows-msvc build exceeds it, C++ goes per-target `default = []` and is opt-in via `cargo build --features cpp`. Verified at the v0.3.0 CI step.

See **[BENCHMARK.md](BENCHMARK.md)** for the method, the reproduce command, and per-mode `recall@k` / `MRR` — across BM25-only, vector-only, hybrid, and hybrid+MMR — on a synthetic corpus, real OSS, and the standard [CodeSearchNet](https://github.com/github/CodeSearchNet) `{NL query → code}` slices (Python/Go/TypeScript, env-pointed, never vendored).

---

## 🏗️ Repository layout

```text
apohara-codesearch/
├── crates/
│   ├── apohara-indexer/        # the engine (library)
│   │   ├── src/
│   │   │   ├── walker.rs        # .gitignore-aware file walk (skips binary/minified)
│   │   │   ├── parser.rs        # tree-sitter structural extraction (9 langs as of v0.3.0)
│   │   │   ├── chunker.rs       # per-symbol + bounded module/window chunks
│   │   │   ├── tokens.rs        # shared snake/camel identifier tokenizer
│   │   │   ├── embeddings.rs    # deterministic blake3 feature-hash vector (default)
│   │   │   ├── embedder.rs      # pluggable Embedder trait + fallback decision
│   │   │   ├── embedder_gemma.rs # EmbeddingGemma-300m in pure candle (opt-in gguf-embed)
│   │   │   ├── storage.rs       # SQLite: chunks + FTS5 + sqlite-vec (dim-parametrized)
│   │   │   ├── schema.rs        # migrations + embedder refuse-to-mix meta
│   │   │   ├── search.rs        # BM25 + vector + RRF + MMR + adaptive weights + boost
│   │   │   ├── incremental.rs   # blake3-delta reindex in one transaction
│   │   │   └── registry.rs      # multi-repo path→index sidecar JSON registry
│   │   ├── tests/
│   │   │   ├── fixtures/         # 9 language fixtures (.rs, .ts, .py, .go, .sh, .java, .c, .rb, .cpp)
│   │   │   └── integration.rs   # hybrid + dedup + ac4 + reindex + incremental
│   │   └── Cargo.toml
│   └── apohara-codesearch/     # the MCP server + CLI
│       ├── src/{main,server,watch,dto}.rs
│       ├── tests/
│       │   ├── corpus_freeze.rs # v0.3.0 corpus freeze guard (drift-fails)
│       │   └── watch_not_a_hook.rs
│       └── examples/           # bench-search (in-CI) · bench-external · bench-codesearchnet · bench-csn-identifier
├── tests/fixtures/             # v0.3.0 frozen corpora (A = full bench, B = 10-query golden set)
├── fuzz/                        # cargo-fuzz targets (parse_source + 6 per-language targets) — isolated crate
├── npm/                         # @apohara/codesearch-mcp wrapper (downloads the Release binary)
├── docs/                        # ASSURANCE.md (assurance case) · best-practices-silver.md (OpenSSF evidence)
├── .omc/plans/                 # RALPLAN-DR plans, scorecard audit, open-questions ledger
├── .clusterfuzzlite/            # ClusterFuzzLite build (Dockerfile + build.sh) for PR fuzzing
├── .claude-plugin/ + marketplace.json   # Claude Code plugin manifest
├── CONTRIBUTING · CODE_OF_CONDUCT · GOVERNANCE · CHANGELOG · SECURITY · deny.toml
└── .github/
    ├── workflows/              # ci · release (cargo-dist) · codeql · scorecard · cflite_pr
    └── dependabot.yml          # weekly cargo + github-actions updates
```

---

## 🗺️ Roadmap

- [x] MCP stdio server (`search_code` + `reindex`) + `watch` subcommand
- [x] Structural extraction for **Rust, TypeScript, Python, Go**
- [x] Hybrid retrieval — BM25 + feature-hash vector, RRF + MMR + structural boost
- [x] Incremental reindex (blake3 content-hash deltas), one SQLite file
- [x] Honest benchmark — synthetic (in-CI) + external real-OSS, with committed known-miss
- [x] Large-OSS soak (Rust + Go ≥100k LOC) — flat ~22 MB peak RSS
- [x] Pluggable `Embedder` trait (opt-in, default stays zero-model)
- [x] Real local embedder backend (candle / safetensors, opt-in, user-supplied)
- [x] Skip generated/minified assets in the walker (DB-bloat hardening)
- [x] Per-language chunk-cap validation (TypeScript / Python)
- [x] **End-to-end robustness over hostile untrusted input** (parser + chunker fuzzed in CI, random/garbage files must not panic the indexer)
- [x] **CodeSearchNet** `recall@5`/`MRR` benchmark — 4 arms (BM25 / vector / hybrid / hybrid+MMR), env-pointed, never vendored
- [x] Adaptive query-shape fusion weighting (opt-in, default off)
- [x] **SLSA Build L3** signed provenance on every release artifact (cargo-dist native attestation)
- [x] Multi-repo schema — composite `PRIMARY KEY(repo_id, path)` + sidecar JSON registry, versioned backward-compatible migration
- [x] `SECURITY.md` threat model + OpenSSF Scorecard workflow
- [x] **Code-trained embedding model — EmbeddingGemma-300m in pure candle** (opt-in `gguf-embed`, user-supplied weights, no native deps, parity 0.99998 vs the official ONNX reference). Measured on CodeSearchNet: the vector arm goes from feature-hash noise (recall@5 0.34/0.005/0.035) to **0.95/0.99/0.885**, hybrid now **beats BM25-only on all 3 slices**, and the adaptive recovery gate (AC4) **closes** — see [BENCHMARK.md](BENCHMARK.md). Default build stays zero-model/offline.
- [x] **OpenSSF Best Practices — Silver** ([#13118](https://www.bestpractices.dev/projects/13118)); every Passing + Silver criterion mapped to evidence in [`docs/best-practices-silver.md`](docs/best-practices-silver.md), with governance (`CONTRIBUTING`/`CODE_OF_CONDUCT`/`GOVERNANCE`/`CHANGELOG`) + an [assurance case](docs/ASSURANCE.md)
- [x] **Supply-chain / OpenSSF Scorecard hardening** — CodeQL SAST (Rust + Actions) · `cargo-fuzz` targets run via ClusterFuzzLite on PRs · `cargo-deny` + `cargo-audit` + Dependabot · all Actions pinned to commit SHAs · least-privilege workflow tokens · branch protection on `main` · signed crates.io + npm publishing
- [x] **v0.3.0 — 9 languages for structural extraction** *(Bash, Java, C, Ruby, C++ added; Kotlin deferred to v0.4.0)*, with corpus freezes, OpenSSF Scorecard **7.0/10** audited, cargo-audit pinned to `Cargo.lock` version
- [x] **v0.3.0 — Per-language fuzz targets** (`parse_bash_source`, `parse_java_source`, `parse_c_source`, `parse_ruby_source`, `parse_cpp_source`, plus the bundled `parse_source` target) so each grammar's extractor is fuzzed independently on PRs
- [x] **v0.3.0 — SymbolKind::Module** added for Ruby `module` declarations
- [x] **v0.3.0 — `SymbolKind::keyword()`** exhaustive for all 8 variants

### v0.4.0 (next)

- [ ] **Kotlin** grammar (either find a `tree-sitter-kotlin 0.23.x` fork or relax the workspace pin to accept 0.3.x)
- [ ] **Server-side bench of `adaptive=true` / `diversify=true`** with the rollback path from `.omc/plans/apohara-codesearch-3frentes.md` §10
- [ ] **OQ-2..OQ-5 v0.8** (AC4 scope, non-gated model mirror, Matryoshka dim, ort vs fastembed) — per-question Pablo sign-off
- [ ] **Story 1 v0.8 — code-trained embedder** (OQ-3 v0.5 follow-up; the real lever for ranking quality)

---

## 🔐 Security

Found a vulnerability? Please report it **privately** via [GitHub Security Advisories](https://github.com/SuarezPM/apohara-codesearch/security/advisories/new) — see [`SECURITY.md`](SECURITY.md) for the disclosure process, supported versions, and the **threat model** (what the tool defends and what is deliberately out of scope). The full **assurance case** (security requirements, trust boundaries, the secure-design argument, and how common weaknesses are countered) is in [`docs/ASSURANCE.md`](docs/ASSURANCE.md).

The project holds the **[OpenSSF Best Practices Silver](https://www.bestpractices.dev/projects/13118)** badge (per-criterion evidence in [`docs/best-practices-silver.md`](docs/best-practices-silver.md)) and is continuously hardened to the **[OpenSSF Scorecard](https://scorecard.dev/viewer/?uri=github.com/SuarezPM/apohara-codesearch)** — measured at **7.0/10** in the v0.3.0 audit (see [`.omc/plans/apohara-codesearch-scorecard-audit.md`](.omc/plans/apohara-codesearch-scorecard-audit.md)):

- **CodeQL** static analysis (Rust + GitHub Actions) on every push/PR;
- **`cargo-fuzz`** targets over the untrusted-input parser/chunker, run via **ClusterFuzzLite** on PRs;
- **`cargo-deny`** (licenses/advisories/sources) + **`cargo-audit`** (RUSTSEC) dependency gates + **Dependabot**;
- all GitHub Actions **pinned to commit SHAs**, **least-privilege** workflow tokens, and **branch protection** on `main`;
- **SLSA Build L3** provenance on every release artifact (verify with `gh attestation verify`), with signed crates.io + npm publishing.

---

## 🤝 Contributing

Contributions are welcome. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the build/test/quality gate, coding standards, testing policy, Conventional Commits, and the DCO sign-off. Participation is governed by the [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) (Contributor Covenant 3.0); how the project is run and how it continues is in [`GOVERNANCE.md`](GOVERNANCE.md); release-by-release changes are in [`CHANGELOG.md`](CHANGELOG.md).

1. **Fork** the repository.
2. Create a feature **branch** (`git checkout -b feature/my-change`).
3. Make your change and run the suite: `cargo test --workspace` (clippy `-D warnings` + `rustfmt --check` gate CI).
4. Open a **pull request** (sign off your commits with `git commit -s`).

> Unless you state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 license, shall be dual-licensed as below, without any additional terms or conditions.

---

## 📄 License

Licensed under either of **[MIT](LICENSE-MIT)** or **[Apache-2.0](LICENSE-APACHE)**, at your option. See [NOTICE](NOTICE) for third-party dependency licenses.

Maintained by **[SuarezPM](https://github.com/SuarezPM)**.
