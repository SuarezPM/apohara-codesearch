# apohara-codesearch

**Hybrid code search for your coding agent — one offline binary, no model, no database.**

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)
[![MCP](https://img.shields.io/badge/MCP-stdio%20server-green.svg)](https://modelcontextprotocol.io)

`apohara-codesearch` is a single Rust binary that runs as a [Model Context
Protocol](https://modelcontextprotocol.io) server, giving a coding agent fast,
fully-offline hybrid search over any local repository. It downloads nothing — no
embedding model, no external vector or graph database — installs in seconds, and
runs air-gapped with a few megabytes of resident memory. The only state it keeps
is one SQLite file.

It is a Claude Code MCP plugin, and works with any MCP client.

## What you get back

A `search_code` hit is a chunk with its structure, not just a line number:

```jsonc
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

Structure (signatures, imports/exports, and `struct`/`enum`/`class`/`type`
symbols) is extracted for **Rust, TypeScript, Python, and Go**. Any other
language is still searchable — it is indexed as overlapping text windows.

## Why a hash, not a model

The dominant code-intelligence tools are heavy: Node plus native bindings, a
C/C++ toolchain for some grammars, an embedded graph or vector database, and a
learned embedding model that downloads on first run. Their strength is deep code
understanding. Their cost is that they are anything but lightweight.

This tool takes the other side of that trade. The embedding is a deterministic
[blake3](https://github.com/BLAKE3-team/BLAKE3) feature-hash, not a learned
model — so there is nothing to download, nothing to serve, and the same input
always produces the same vector. That makes the semantic recall weaker than a
model-based tool; we compensate with hybrid retrieval rather than pretending the
hash is semantic.

|                          | apohara-codesearch                      | graph / embedding tools            | ripgrep        |
| ------------------------ | --------------------------------------- | ---------------------------------- | -------------- |
| Runtime dependencies     | one static binary                       | Node + native bindings + toolchain | one binary     |
| Model download           | none                                    | hundreds of MB                     | none           |
| External DB / service    | none                                    | embedded graph / vector DB         | none           |
| Offline / air-gapped     | ✓                                       | usually requires a fetch           | ✓              |
| Structural context       | signatures + imports/exports (4 langs)  | call graphs, deep                  | text only      |
| Ranking                  | hybrid BM25 + vector (RRF)              | learned embeddings                 | exact / regex  |

Lighter than the graph tools, structure-aware where `ripgrep` is text-only. It
does **not** match a model-based tool on conceptual recall, and it does not build
a call graph — those are deliberately out of scope.

## Quick start

Register it with your MCP client. For Claude Code, add to `.mcp.json`:

```json
{ "mcpServers": { "codesearch": { "command": "npx", "args": ["-y", "@apohara/codesearch-mcp"] } } }
```

The `npx` wrapper downloads the right prebuilt binary for your platform on first
run. Prefer to build from source, or run the binary directly:

```bash
cargo install --path crates/apohara-codesearch   # from a checkout
# or run the binary directly as an MCP server:
apohara-codesearch serve
```

That is the whole install. No model, no database, no daemon.

## Tools

| Tool          | What it does                                                                                        |
| ------------- | --------------------------------------------------------------------------------------------------- |
| `search_code` | Hybrid BM25 + vector search over a repo path. Lazily indexes on first call. Returns the top-k hits. |
| `reindex`     | Re-index a repo. Incremental by default (blake3 content-hash deltas); `force: true` rebuilds.       |

There is also a `watch` subcommand — `apohara-codesearch watch <path>` keeps the
index current as files change. It is a plain CLI loop, not a plugin hook.

## How it works

1. **Walk + chunk.** A `.gitignore`-aware walk splits each file into per-symbol
   chunks (with the symbol's signature attached) plus bounded module-remainder
   and window chunks, so a giant file never collapses into one diluted chunk.
2. **Index.** Each chunk gets a BM25 lexical row (SQLite FTS5) and a feature-hash
   vector row (sqlite-vec), keyed on a shared row id. The lexical and vector
   sides share one identifier tokenizer, so `parseString` and `parse_string`
   match each other.
3. **Search.** A query runs through both BM25 and vector k-NN; the two ranked
   lists are merged with [Reciprocal Rank
   Fusion](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf), then the
   surviving chunks are hydrated with their structural context.
4. **Stay current.** Re-indexing hashes each file and reprocesses only what
   changed, in a single transaction that keeps the three tables consistent.

## Footprint at scale

Measured indexing [tokio](https://github.com/tokio-rs/tokio) (174k LOC of Rust)
with the default feature-hash embedder on a desktop machine:

| Metric               | Value                           |
| -------------------- | ------------------------------- |
| Cold index           | ~9 s (835 files, 16,400 chunks) |
| Peak resident memory | ~22 MB                          |
| Warm query latency   | ~10 ms                          |
| Index on disk        | ~40 MB (one SQLite file)        |

No out-of-memory, no external process. See [BENCHMARK.md](BENCHMARK.md) for the
method, the reproduce command, and a retrieval-quality comparison across
BM25-only, vector-only, and hybrid modes.

## Honest limitations

- **The vector is a robustness layer, not a semantic engine.** Because the
  embedding is a feature-hash, a conceptual query that shares no tokens with the
  target will not surface it. On a clean corpus where lexical search already
  wins, fusion can even be a slight net negative — [BENCHMARK.md](BENCHMARK.md)
  reports this rather than hiding it.
- **Deep structural context is out of scope.** It returns signatures and
  file-level imports/exports, not callers/callees or a call graph.
- **Optional upgrade path.** A local embedding model can be enabled as an opt-in
  build feature; the model file is user-supplied and never downloaded, so the
  default install stays zero-dependency.

## License

Licensed under either of

- **MIT license** ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)

at your option. See [NOTICE](NOTICE) for third-party dependency licenses.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
