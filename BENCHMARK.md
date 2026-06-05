# Benchmarks

> Honest, reproducible numbers — not marketing. We do **not** put a self-graded
> recall figure in the README headline; the numbers live here, with the method
> stated so you can judge them yourself, and the harness is in-repo so you can
> re-run it.

## Methodology (read this before trusting any number)

- **Corpus:** a small, checked-in, multi-language synthetic corpus under
  `examples/bench-corpus/`. It is authored by the tool's author, so treat the
  absolute numbers with appropriate skepticism — the *method*, not the corpus,
  is the credibility anchor.
- **Labeling rule (frozen before running the tool):** a chunk is *relevant* to a
  query if it is the chunk a human would open to answer that query. Relevance is
  recorded by `(file, target symbol / line region)`, **not** by exact
  `path:start-end` chunk id, so labels do not go stale when chunk boundaries
  change.
- **Adversarial / anti-circular:** **≥ 30%** of the labeled queries are committed
  *known-miss* cases — queries whose relevant chunk the hybrid pipeline ranks
  *outside* top-k, or finds only via one mode. Publishing the tool's own failures
  is the guard against a self-flattering corpus.
- **Modes reported (so you see the honest picture, not just wins):** BM25-only,
  vector-only, and hybrid (RRF). We report `recall@k` and `MRR` for each, AND the
  count of queries where **hybrid is worse than the best single mode** — not only
  where it wins.
- **Determinism:** `recall@k` / `MRR` are byte-stable across runs (deterministic
  feature-hash + deterministic RRF tie-break). Peak RSS and wall-clock are *not*
  part of the determinism claim — they are live measurements.

## Reproduce

```bash
cargo run --release --example bench-search
```

The binary indexes `examples/bench-corpus/` into a throwaway temp database, runs
every labeled query through all three modes, applies the file+line relevance
rule, and prints the table below. It measures the whole set twice and asserts the
two metric sets are byte-identical before printing, so a nondeterminism
regression fails the run.

## Results — synthetic corpus (in-CI)

Corpus: 18 hand-written source files (7 Rust, 5 TypeScript, 3 Python, 3 Go) plus
3 unparsed text files (`docs/architecture.md`, `CHANGELOG.txt`, `README.md`) — 21
files total, indexing to 194 chunks and 99 extracted symbols.

| Mode | recall@5 | recall@10 | MRR |
|------|----------|-----------|-----|
| BM25-only | 0.542 | 0.625 | 0.326 |
| vector-only | 0.083 | 0.083 | 0.063 |
| hybrid (RRF) | 0.458 | 0.542 | 0.285 |

- Queries: 24 (of which 9 — 38% — are committed known-miss, above the 30% floor)
- Queries where hybrid < best single mode: 9

**Read this honestly.** On *this* corpus the hybrid does **not** beat BM25-alone —
it is slightly worse on every metric, and on 9 of 24 queries fusion *demotes* the
relevant chunk below where BM25 ranked it. The cause is structural, not a bug: the
default embedder is a deterministic feature-hash (no learned model), so its
ranked list is near-random for natural-language intent — vector-only recall@5 is
0.083. Reciprocal rank fusion then mixes that weak list into the strong lexical
one and pays a fusion tax. The honest takeaway is the one the README already
states: the vector side is a *robustness layer*, not a semantic engine, and on a
small clean corpus where lexical search already wins, fusion is a net negative.
The separate `rrf_beats_bm25_alone` regression test proves fusion *can* lift a
crafted hard case above BM25-alone; this corpus shows that is not the average
case for the feature-hash backend. A learned embedder (the `gguf-embed` feature)
is the lever expected to flip this, and re-running this harness is how that claim
would be checked — not asserted.

## Real-OSS soak (one-off, not in CI)

> A large real repository indexed locally to measure footprint at scale.
> Corpus NOT vendored.

| Repo | LOC | Files indexed | Chunks | Index time (cold, force) | Peak RSS (indexing) | Query latency (warm) | Index DB on disk |
|------|-----|---------------|--------|--------------------------|---------------------|----------------------|------------------|
| [tokio](https://github.com/tokio-rs/tokio) (shallow clone) | 174,542 (.rs) | 835 | 16,400 | 9.0 s | ~22 MB | ~10 ms | 40 MB (single SQLite file) |

Measured on a Ryzen 5 3600 / 48 GB box with the default (feature-hash) embedder,
release binary, driven over the stdio MCP `reindex` + `search_code` tools.

Notes:
- **No OOM, no panic.** Peak resident memory while indexing a 174k-LOC repo is
  ~22 MB; a warm query peaks at ~11 MB. The single 40 MB SQLite file is the only
  state — no external DB, no model. This is the "near-zero resident RAM" claim,
  measured.
- The warm query `"mutex lock guard poison"` and the indexing-time query
  `"spawn async task on the runtime scheduler"` both returned correct,
  on-topic tokio runtime/sync chunks with file/line + imports.
- No chunking pathology surfaced on tokio (no giant-chunk dilution or runaway
  module remainders) — the Phase-1 chunk cap + bounded module split hold at scale.
