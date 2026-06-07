<div align="center">

# apohara-codesearch

**Hybrid code search for your coding agent — _one offline binary, no model, no database._**

[![CI](https://img.shields.io/github/actions/workflow/status/SuarezPM/apohara-codesearch/ci.yml?style=for-the-badge&label=CI)](https://github.com/SuarezPM/apohara-codesearch/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=for-the-badge)](#-license)
[![Rust](https://img.shields.io/badge/rust-stable-orange?style=for-the-badge&logo=rust)](https://www.rust-lang.org)
[![npm](https://img.shields.io/npm/v/@apohara/codesearch-mcp?style=for-the-badge&label=npm&color=purple)](https://www.npmjs.com/package/@apohara/codesearch-mcp)
[![MCP](https://img.shields.io/badge/MCP-stdio%20server-success?style=for-the-badge)](https://modelcontextprotocol.io)

[![OpenSSF Scorecard](https://img.shields.io/ossf-scorecard/github.com/SuarezPM/apohara-codesearch?style=for-the-badge&label=Scorecard)](https://scorecard.dev/viewer/?uri=github.com/SuarezPM/apohara-codesearch)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/13118/badge)](https://www.bestpractices.dev/projects/13118)

**[Quick Start](#-quick-start)** · **[Features](#-features)** · **[Where it fits](#-where-it-fits)** · **[How it works](#-how-it-works--honesty)**

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

> Structure (signatures, imports/exports, and `struct`/`enum`/`class`/`type` symbols) is extracted for **Rust, TypeScript, Python, and Go**. Any other language is still searchable — indexed as overlapping text windows.

---

## 💡 Concept

> [!NOTE]
> **A hash, not a model.** The dominant code-intelligence tools are heavy: Node plus native bindings, a C/C++ toolchain for some grammars, an embedded graph or vector database, and a learned embedding model that downloads on first run. Their strength is deep understanding; their cost is that they are anything but lightweight.

`apohara-codesearch` takes the other side of that trade. The embedding is a deterministic [blake3](https://github.com/BLAKE3-team/BLAKE3) feature-hash, **not** a learned model — so there is nothing to download, nothing to serve, and the same input always produces the same vector. That makes semantic recall weaker than a model-based tool; we compensate with **hybrid retrieval** (lexical + vector, fused) rather than pretending the hash is semantic. It is a Claude Code MCP plugin, and works with any MCP client.

---

## ✨ Features

| | |
|---|---|
| 🔌 **MCP stdio server** | Two tools — `search_code` (hybrid) and `reindex` (incremental) — over plain JSON-RPC. Works with Claude Code or any MCP client. |
| 🦀 **One static binary** | No Node, no native bindings, no toolchain, no service. `cargo install` or `npx`, then run. The only state is one SQLite file. |
| 🧠 **Hybrid ranking** | BM25 (SQLite FTS5) + a feature-hash vector (sqlite-vec), merged with Reciprocal Rank Fusion, then MMR-diversified. Optional **adaptive weighting** biases the fusion by query shape (opt-in, off by default). |
| 🌳 **Structural extraction** | Per-symbol chunks with signatures + file imports/exports for **Rust, TS, Python, Go**; everything else indexed as text. |
| 📴 **Offline & air-gapped** | Zero network at runtime AND at build. No model fetch, no telemetry, no API keys. |
| 🪶 **Near-zero footprint** | ~22 MB resident memory indexing a 224k-LOC repo — flat with repo size (memory-bounded pipeline). |
| 🗂️ **Multi-repo aware** | Each repo keeps its own SQLite index (`PRIMARY KEY(repo_id, path)`); a sidecar JSON registry tracks the path→index map, with versioned, backward-compatible migrations. |
| ⚡ **Incremental + watch** | `reindex` does blake3 content-hash deltas; the `watch` subcommand keeps the index current as files change (a plain CLI loop, **not** a plugin hook). |
| 🔁 **Deterministic** | Same input ⇒ same vector ⇒ byte-stable `recall@k`/`MRR`. Re-indexing is stable. |
| 🔏 **Signed releases** | Every release artifact carries a [SLSA Build L3](https://slsa.dev) provenance attestation; see [SECURITY.md](SECURITY.md) for the threat model. |

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
# Build + install from a checkout (lowest-trust path):
cargo install --path crates/apohara-codesearch

# Run the binary directly as a stdio MCP server:
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
| **Structural context** | signatures + imports/exports (4 langs) | call graphs, deep | text only |
| **Ranking** | hybrid BM25 + vector (RRF) | learned embeddings | exact / regex |

---

## 🔬 How it works / honesty

1. **Walk + chunk.** A `.gitignore`-aware walk splits each file into per-symbol chunks (with the symbol's signature attached) plus bounded module-remainder and window chunks, so a giant file never collapses into one diluted chunk.
2. **Index.** Each chunk gets a BM25 lexical row (SQLite FTS5) and a feature-hash vector row (sqlite-vec), keyed on a shared row id. Both sides share one identifier tokenizer, so `parseString` and `parse_string` match each other.
3. **Search.** A query runs through both BM25 and vector k-NN; the two ranked lists are merged with [Reciprocal Rank Fusion](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf), diversified with MMR, then the survivors are hydrated with their structural context.
4. **Stay current.** Re-indexing hashes each file and reprocesses only what changed, in a single transaction that keeps the three tables consistent.

### Footprint at scale

Measured with the default feature-hash embedder on a Ryzen 5 3600 / 48 GB box, driven over the stdio MCP tools:

| Repo | LOC | Cold index | Peak RSS | Warm query | Index on disk |
|---|---|---|---|---|---|
| [tokio](https://github.com/tokio-rs/tokio) | 174k Rust | ~10 s | ~22 MB | ~18 ms | 39 MB |
| [hugo](https://github.com/gohugoio/hugo) | 224k Go | ~26 s | ~24 MB | ~22 ms | 54 MB |

Peak resident memory is **flat across repo size** — no OOM, no external process. One SQLite file is the only state.

> [!WARNING]
> **The vector is a robustness layer, not a semantic engine.** Because the embedding is a feature-hash, a conceptual query that shares no tokens with the target will not surface it — and on a clean corpus where lexical search already wins, fusion can be a slight net negative. [BENCHMARK.md](BENCHMARK.md) **publishes this** (synthetic corpus + a one-off external comparison on real OSS, with ≥30% committed known-miss queries) rather than hiding it. Deep structural context (callers/callees, call graphs) is out of scope by design. A real local embedding model is an **opt-in, user-supplied** build feature — never downloaded — so the default install stays zero-dependency. **That lever now exists and is measured:** the `gguf-embed` feature runs **EmbeddingGemma-300m in pure candle** (no native deps) on user-supplied weights, and on CodeSearchNet it flips this picture entirely — the vector arm beats BM25-only and fusion stops being a tax (see [BENCHMARK.md](BENCHMARK.md), v0.8 section). The default build is unchanged: still the deterministic feature-hash, still zero-model.

See **[BENCHMARK.md](BENCHMARK.md)** for the method, the reproduce command, and per-mode `recall@k` / `MRR` — across BM25-only, vector-only, hybrid, and hybrid+MMR — on a synthetic corpus, real OSS, and the standard [CodeSearchNet](https://github.com/github/CodeSearchNet) `{NL query → code}` slices (Python/Go/TypeScript, env-pointed, never vendored).

---

## 🏗️ Repository layout

```text
apohara-codesearch/
├── crates/
│   ├── apohara-indexer/        # the engine (library)
│   │   └── src/
│   │       ├── walker.rs        # .gitignore-aware file walk
│   │       ├── parser.rs        # tree-sitter structural extraction (Rust/TS/Python/Go)
│   │       ├── chunker.rs       # per-symbol + bounded module/window chunks
│   │       ├── tokens.rs        # shared snake/camel identifier tokenizer
│   │       ├── embeddings.rs    # deterministic blake3 feature-hash vector
│   │       ├── embedder.rs      # pluggable Embedder trait (opt-in gguf-embed)
│   │       ├── storage.rs       # SQLite: chunks + FTS5 + sqlite-vec
│   │       ├── schema.rs        # migrations + embedder refuse-to-mix meta
│   │       ├── search.rs        # BM25 + vector + RRF + MMR + adaptive weights + boost
│   │       ├── incremental.rs   # blake3-delta reindex in one transaction
│   │       └── registry.rs      # multi-repo path→index sidecar JSON registry
│   └── apohara-codesearch/     # the MCP server + CLI
│       ├── src/{main,server,watch,dto}.rs
│       └── examples/           # bench-search (in-CI) · bench-external · bench-codesearchnet (one-off)
├── npm/                         # @apohara/codesearch-mcp wrapper (downloads the Release binary)
├── .claude-plugin/ + marketplace.json   # Claude Code plugin manifest
└── .github/workflows/          # ci.yml (test/clippy/fmt/dist) · release.yml (cargo-dist)
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
- [x] **CodeSearchNet** `recall@5`/`MRR` benchmark — 4 arms (BM25 / vector / hybrid / hybrid+MMR), env-pointed, never vendored
- [x] Adaptive query-shape fusion weighting (opt-in, default off)
- [x] **SLSA Build L3** signed provenance on every release artifact (cargo-dist native attestation)
- [x] Multi-repo schema — composite `PRIMARY KEY(repo_id, path)` + sidecar JSON registry, versioned backward-compatible migration
- [x] `SECURITY.md` threat model + OpenSSF Scorecard workflow
- [x] **Code-trained embedding model — EmbeddingGemma-300m in pure candle** (opt-in `gguf-embed`, user-supplied weights, no native deps, parity 0.99998 vs the official ONNX reference). Measured on CodeSearchNet: the vector arm goes from feature-hash noise (recall@5 0.34/0.005/0.035) to **0.95/0.99/0.885**, hybrid now **beats BM25-only on all 3 slices**, and the adaptive recovery gate (AC4) **closes** — see [BENCHMARK.md](BENCHMARK.md). Default build stays zero-model/offline.
- [x] **OpenSSF Best Practices** — enrolled ([#13118](https://www.bestpractices.dev/projects/13118)); Passing + Silver criteria mapped to evidence in [`docs/best-practices-silver.md`](docs/best-practices-silver.md), with governance (`CONTRIBUTING`/`CODE_OF_CONDUCT`/`GOVERNANCE`/`CHANGELOG`), an [assurance case](docs/ASSURANCE.md), and supply-chain CI (`cargo-deny` + `cargo-audit` + Dependabot + offline-isolation guard)

---

## 🔐 Security

Found a vulnerability? Please report it **privately** via [GitHub Security Advisories](https://github.com/SuarezPM/apohara-codesearch/security/advisories/new) — see [`SECURITY.md`](SECURITY.md) for the disclosure process, supported versions, and the **threat model** (what the tool defends and what is deliberately out of scope). The full **assurance case** (security requirements, trust boundaries, the secure-design argument, and how common weaknesses are countered) is in [`docs/ASSURANCE.md`](docs/ASSURANCE.md). Supply-chain health is tracked by an [OpenSSF Scorecard](https://scorecard.dev/viewer/?uri=github.com/SuarezPM/apohara-codesearch) workflow and the [OpenSSF Best Practices](https://www.bestpractices.dev/projects/13118) badge; the per-criterion evidence map is in [`docs/best-practices-silver.md`](docs/best-practices-silver.md).

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
