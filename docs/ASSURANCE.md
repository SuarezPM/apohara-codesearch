# Assurance Case

This is apohara-codesearch's **assurance case**: a structured argument for *why
its security requirements are met*. It states the security requirements, the
threat model and trust boundaries, the secure-design principles applied, and how
common implementation weaknesses are countered — each with pointers to the code,
tests, and CI that back the claim. It consolidates and does not restate
[`SECURITY.md`](../SECURITY.md) (the threat model in full); where the two differ,
`SECURITY.md` wins on the threat model and this document wins on the design
argument.

## 1. Security requirements (what we promise)

apohara-codesearch is a **local, offline** code-search tool: a single Rust binary
that indexes and searches source code on the user's own machine and speaks the
Model Context Protocol over local stdio. Its security requirements:

1. **No code execution of indexed content.** The tool reads and parses source as
   text/AST; it **never compiles, evaluates, or runs** the code it indexes.
2. **Robust parsing of untrusted files.** A crafted, malformed, binary, or
   pathological input file does not crash the process unrecoverably, hang it,
   exhaust memory, or corrupt the on-disk index.
3. **Stay inside the indexed tree.** The walker does not follow symlinks out of
   the tree or otherwise read files outside the directory the user pointed it at.
4. **No network attack surface.** The index/query/watch paths make **no outbound
   network connections**; the default build links **no HTTP client**.
5. **Local index integrity.** The single SQLite index file remains consistent
   across incremental reindex and schema migration; a malformed input cannot
   silently corrupt it.

Non-requirements (explicitly out of scope, documented so they are not
over-read): defending a host or MCP client that is **already compromised**;
treating "the repo I indexed contained malware" as a tool vulnerability (indexing
does not execute it); and performance/memory growth on legitimately huge but
trusted inputs. See
[`SECURITY.md`](../SECURITY.md#explicitly-out-of-scope-by-design).

## 2. Threat model and trust boundaries

### Actors and assets
- **User** (trusted): runs the binary locally with their own privileges.
- **Indexed repository** (untrusted *data*): may contain hostile, malformed, or
  adversarially crafted files — that is the whole point of a code-search tool.
- **MCP client** (trusted): a coding agent the user already chose to run, driving
  the server over local stdio.
- **Assets:** process liveness/integrity while parsing untrusted files, the
  integrity of the local SQLite index, and confinement of reads to the indexed
  tree.

### Trust boundaries
- **Boundary A — untrusted file content.** Every byte of an indexed file is
  attacker-influenced. It is only ever read, parsed (tree-sitter), tokenized, and
  hashed — never executed. Parsing is error-tolerant (see §4).
- **Boundary B — the filesystem walk.** The walk is rooted at the user-supplied
  path and uses `ignore::WalkBuilder` with `follow_links` left at its default of
  **false** ([`walker.rs`](../crates/apohara-indexer/src/walker.rs)), so a symlink
  inside the tree is not traversed out of it.
- **Boundary C — the MCP surface (local stdio).** Tool arguments (`path`,
  `query`) are attacker-influenceable through the agent; they drive a bounded
  read/search, never a shell-out or an eval.
- **No network boundary.** There is no remote actor in the normal operating
  model: nothing is fetched and no socket is opened for indexing or querying.

### Threats considered and mitigations
| Threat | Mitigation |
|--------|------------|
| Crafted file crashes/hangs the indexer | error-tolerant tree-sitter parse (no `unwrap` on parse result); unsupported/unparsable content falls back to bounded text windows ([`chunker.rs`](../crates/apohara-indexer/src/chunker.rs)); non-UTF-8 split points respect char boundaries ([`storage.rs` `is_char_boundary`](../crates/apohara-indexer/src/storage.rs)) |
| One giant/minified line bloats the index | the walker skips generated/minified assets by maximum line length ([`walker.rs`](../crates/apohara-indexer/src/walker.rs)); per-symbol + bounded module/window chunk caps ([`chunker.rs`](../crates/apohara-indexer/src/chunker.rs)) |
| Symlink read escapes the indexed tree | `WalkBuilder` does not follow symlinks (default `follow_links = false`) |
| Indexed code is executed | it never is — the tool only reads/parses; there is no `eval`/compile/exec path anywhere in the tree |
| Network adversary | the index/query/watch paths open no socket and link no HTTP client (**CI-enforced**, §4) |
| Malformed input corrupts the index | reindex runs in a single transaction that keeps the FTS5 + vector + chunks tables consistent ([`incremental.rs`](../crates/apohara-indexer/src/incremental.rs)); a migration that would lose data is gated loudly ([`schema.rs`](../crates/apohara-indexer/src/schema.rs)) |
| Index built with a different embedder is silently mixed | refuse-to-mix guard rejects an index whose stored embedder id/dim differs ([`schema.rs` `verify_embedder_meta`](../crates/apohara-indexer/src/schema.rs)) |
| Tampered prebuilt binary (separate acquisition path) | release artifacts carry SLSA Build L3 provenance (Sigstore keyless); `gh attestation verify` checks them |

## 3. Secure-design principles applied

- **No execution of untrusted input.** Indexed content is data, never code. This
  removes the single largest class of risk a "tool that points at arbitrary
  repos" could otherwise have.
- **Fail soft on bad input, not closed-then-crash.** Unsupported languages and
  unparsable files degrade to text-window indexing rather than aborting; the hit
  shape stays well-formed ([`tests/integration.rs` `ac4_hit_shape_and_graceful_degradation`](../crates/apohara-indexer/tests/integration.rs)).
- **Least surface.** No daemon, no server socket, no external database, no
  credentials, no telemetry. The only state is one SQLite file; the only
  transport is stdio.
- **Offline by construction.** The default build pulls no model and no HTTP
  client; a real embedding model is an opt-in, user-supplied, local-path feature
  that is never downloaded ([`embedder.rs`](../crates/apohara-indexer/src/embedder.rs)).
- **Memory safety with a small, audited `unsafe` surface.** The shipped code is
  safe Rust except for two narrow, necessary spots, neither doing pointer
  arithmetic on attacker data:
  - **FFI registration of the sqlite-vec extension** via
    `sqlite3_auto_extension` ([`storage.rs`](../crates/apohara-indexer/src/storage.rs)) —
    a one-time C-ABI handshake at startup (default build).
  - **`mmap` of user-supplied safetensors weights** in the opt-in `gguf-embed`
    embedder ([`embedder_gemma.rs`](../crates/apohara-indexer/src/embedder_gemma.rs)) —
    not present in the default build.
- **Determinism.** Same input ⇒ same vector ⇒ stable results; reindex is a pure
  function of file content (blake3 deltas), which makes behavior reproducible and
  auditable.

## 4. Common implementation weaknesses — countered

- **Input validation (untrusted input).** The two untrusted input classes are
  handled explicitly. (a) **File content** has no required format — any bytes are
  acceptable to index; structure is *extracted where possible* (tree-sitter, which
  is error-tolerant and yields `ERROR` nodes rather than panicking) and otherwise
  the file is indexed as bounded text windows, so there is no parse path that
  rejects-by-crash. (b) **MCP tool arguments** (`path`, `query`) drive a bounded
  read/search rooted at the given path; an empty or absent path/query is handled,
  not assumed. Non-UTF-8 and multibyte boundaries are respected when slicing
  ([`storage.rs`](../crates/apohara-indexer/src/storage.rs)).
- **No network surface.** The index/query/watch paths make no outbound
  connection. A CI job (`offline-isolation`) asserts the default
  normal-dependency tree links **no** HTTP client
  (reqwest/hyper/ureq/curl/hf-hub/isahc/attohttpc), so an accidental future
  dependency cannot add a network surface ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)).
- **Dependency risk.** `cargo-deny` (licenses/bans/advisories/sources, evaluated
  with `all-features` so the opt-in candle tree is covered) and `cargo-audit`
  (RUSTSEC) run in CI on every push/PR; the single documented exception
  (`RUSTSEC-2024-0436`, `paste` unmaintained, opt-in tree only) is justified in
  [`deny.toml`](../deny.toml). Dependabot opens update PRs weekly.
- **Static analysis.** `clippy` with `-D warnings` (no warning tolerated) on every
  change, plus an OpenSSF Scorecard workflow feeding the supply-chain badge.
- **Index integrity.** Reindex is a single transaction; schema migrations are
  idempotent and gate a data-losing downgrade loudly ([`schema.rs`](../crates/apohara-indexer/src/schema.rs)).

## 5. Residual risk (honest)

- **Local trust is assumed.** The tool runs with the user's privileges and is not
  a sandbox; a compromised host or MCP client is out of scope.
- **A determined adversarial input is a fuzzing surface.** Parsing is
  error-tolerant and bounded, but the project does not yet run continuous fuzzing;
  pathological-input reports are explicitly welcomed in `SECURITY.md`.
- **The opt-in embedder adds an `mmap`/`unsafe` and a heavier dependency tree.**
  It is off by default and loads only user-supplied local weights.

These are documented, intentional limitations, not undisclosed gaps.

## 6. Evidence index

| Claim | Evidence |
|-------|----------|
| No code execution | no eval/compile/exec path in the tree; design (§1, §3) |
| Robust parse / graceful degradation | `crates/apohara-indexer/tests/integration.rs` (`ac4_hit_shape_and_graceful_degradation`), `chunker.rs` window-strategy tests |
| Walk stays in tree | `walker.rs` (`WalkBuilder`, default `follow_links=false`); `test_walk_repo_honors_gitignore` |
| No network surface | CI `offline-isolation` job; `cargo tree -e normal` links no HTTP client |
| Index integrity across reindex/migration | `tests/integration.rs` (`ac7_incremental_reindex_no_fts5_error`), `tests/persistence_reopen.rs`, `schema.rs` migration tests |
| Dependency hygiene | CI `deny` + `audit` jobs; `deny.toml`; `.github/dependabot.yml` |
| Static analysis | CI `clippy -D warnings`; `scorecard.yml` |
| Test coverage | ≈89% line coverage (`cargo llvm-cov --workspace --summary-only`); see [`best-practices-silver.md`](best-practices-silver.md) |
| Signed releases | `.github/workflows/release.yml` (cargo-dist native attestation); `gh attestation verify` |
