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
| [tokio](https://github.com/tokio-rs/tokio) (shallow clone) | 174,542 (.rs) | 835 | 16,400 | 9.8 s | ~21.5 MB | ~18 ms | 39 MB (single SQLite file) |
| [hugo](https://github.com/gohugoio/hugo) (shallow clone) | 224,209 (.go) | 2,270 | 21,361 | 26.1 s | ~23.5 MB | ~22 ms | 54 MB (single SQLite file) |

Measured on a Ryzen 5 3600 / 48 GB box with the default (feature-hash) embedder,
release binary, driven over the stdio MCP `reindex` + `search_code` tools. Each
repo indexed twice (cold) to bound RSS sampling noise: peak RSS varied < 2.5 %
run-to-run (tokio 21.7/22.2 MB, hugo 24.1/23.6 MB), so a +10 % footprint ceiling
sits comfortably above measurement jitter.

Notes:
- **No OOM, no panic.** Peak resident memory while indexing a 174k-LOC repo is
  ~22 MB; a warm query peaks at ~11 MB. The single 40 MB SQLite file is the only
  state — no external DB, no model. This is the "near-zero resident RAM" claim,
  measured.
- The warm query `"mutex lock guard poison"` and the indexing-time query
  `"spawn async task on the runtime scheduler"` both returned correct,
  on-topic tokio runtime/sync chunks with file/line + imports.
- **Peak RSS is flat across repo size** (~22 MB for ripgrep 52k LOC, tokio 174k,
  and hugo 224k) — resident memory does not scale with the repo, confirming the
  index pipeline is memory-bounded (the "near-zero resident RAM" claim holds at
  scale on a 224k-LOC Go repo, not just Rust).
- **tokio (Rust): no chunking pathology.** The largest chunks are legitimate
  whole symbols (e.g. `tokio-stream/src/stream_ext.rs` `StreamExt` trait, 1106
  lines / 38 KB — stored whole, truncated only for FTS/embed); `module`/`window`
  chunks respect `MAX_CHUNK_BYTES` (max ~8187 B). The Phase-1 cap + bounded
  module split hold.
- **hugo (Go): one chunking pathology found (documented, fix deferred).**
  Minified/generated JS assets in the repo are indexed as oversized `window`
  chunks: `internal/warpc/js/renderkatex.bundle.js` → a 277 KB chunk (36 lines),
  `livereload/livereload.min.js` → 80 KB (7 lines). `MAX_CHUNK_LINES=200` does
  not split them (few lines, each enormously long) and `MAX_CHUNK_BYTES` only
  truncates the *indexed* (FTS/embed) text, so the full minified blob is still
  stored in `chunks.body`, bloating the DB with non-source noise. Only 27 of
  21,361 chunks exceed 8 KB, so it is bounded — no OOM, no panic — but it is real
  index bloat. **Follow-up (own impact analysis, out of US-3 measurement scope):**
  skip generated/minified assets in the walker (heuristic: very high bytes-per-line),
  or cap the stored `body`. Tracked for a future hardening pass.

## External real-OSS comparison (one-off, not in CI, corpus not vendored)

> Recall/MRR over hand-labeled developer queries against two REAL third-party
> repositories on local disk. Corpus NOT vendored — the repos are cloned once,
> manually, outside this repo (a one-time online prereq, NOT part of any build,
> test, or the tool's runtime). The harness reads them ONLY from local disk.

**Pinned commits (reproducibility):**

| Repo | License | SHA |
|------|---------|-----|
| [ripgrep](https://github.com/BurntSushi/ripgrep) (Rust, ~52k LOC) | MIT/Unlicense | `82313cf95849bfe425109ad9506a52154879b1b1` |
| [hugo](https://github.com/gohugoio/hugo) (Go, ~224k LOC) | Apache-2.0 | `7d1b1fb33dd7bdbb0d16dde9509ce15d93f7d894` |

**Harness:** `crates/apohara-codesearch/examples/bench-external.rs` — an *example*
(never packaged by dist: `dist plan | grep -c bench-external` == 0), mirroring the
in-CI synthetic `bench-search.rs` exactly (same three modes, same `recall@5`/
`recall@10`/MRR, same "hybrid worse than best single mode" count, same twice-run
byte-identical determinism assertion). Same frozen honesty contract: relevance is
`(file, target_line)` — NOT a `path:start-end` chunk id — so a label survives a
chunk-boundary change (this is what lets US-4 sweep the caps without re-labeling).
Run it with:

```bash
APOHARA_BENCH_EXTERNAL_ROOT=/path/to/parent-of-clones \
  cargo run --release --example bench-external
```

where `APOHARA_BENCH_EXTERNAL_ROOT` holds the pre-cloned `ripgrep/` and `hugo/`
checkouts (unset → the harness prints guidance and exits 0). Default (feature-hash)
embedder, release binary.

### ripgrep (Rust) slice — 22 labeled queries, 16 known-miss (73%)

| Mode | recall@5 | recall@10 | MRR |
|------|----------|-----------|-----|
| BM25-only | 0.273 | 0.409 | 0.1494 |
| vector-only | 0.045 | 0.045 | 0.0455 |
| hybrid (RRF) | 0.227 | 0.273 | 0.1191 |

- Queries: 22 (16 — **73%** — committed known-miss, far above the 30% floor)
- Queries where hybrid < best single mode: **8**

### hugo (Go) slice — 9 labeled queries, 7 known-miss (78%)

| Mode | recall@5 | recall@10 | MRR |
|------|----------|-----------|-----|
| BM25-only | 0.222 | 0.444 | 0.2531 |
| vector-only | 0.000 | 0.000 | 0.0000 |
| hybrid (RRF) | 0.222 | 0.222 | 0.2389 |

- Queries: 9 (7 — **78%** — committed known-miss, far above the 30% floor)
- Queries where hybrid < best single mode: **2**

This is the non-Rust signal US-4 needs: the GLOBAL `MAX_CHUNK_LINES`/
`MAX_CHUNK_BYTES` caps are exercised on a Go corpus too, so a cap sweep cannot be
tuned on a Rust-only signal. (TS/Python remain unmeasured — OQ-5.)

**Determinism:** metrics are byte-stable. The harness measures each slice twice
and asserts the two metric sets identical before printing; two *separate* process
invocations also produce byte-identical stdout (verified by `sha256sum`).

### Read this honestly

On real OSS the feature-hash backend is **weak**, and far weaker than on the small
synthetic corpus — exactly as the README's "robustness layer, not a semantic
engine" claim predicts, now measured at repo scale:

- **The hybrid never beats BM25-alone** on either slice; it is equal-or-worse on
  every metric, and on 8/22 (ripgrep) + 2/9 (hugo) queries fusion *demotes* the
  relevant chunk below where BM25 ranked it. Several BM25 wins (e.g. ripgrep
  `parse_low` at rank 6, `matched_stripped` at rank 7; hugo `Convert` at rank 6,
  `RootMappingFs.Open` at rank 9) are pushed *out of the top-10* by RRF mixing in
  the near-random vector list. This is the same fusion-tax the synthetic section
  documents, amplified by repo size.
- **Vector-only is almost useless:** recall@5 = 0.045 on ripgrep (1 of 22 queries
  — the JSON-serialize one), 0.000 on hugo. The deterministic feature-hash maps a
  query to a fixed point with no learned semantics, so on a 52k–224k-LOC repo its
  KNN list is effectively noise. It contributes a robustness signal (it never
  *crashes* and degrades gracefully when FTS misses), not a recall signal.
- **The known-miss rate is high BECAUSE the labels are honest.** Most misses are
  not exotic: common code tokens (`search`, `path`, `match`, `glob`, `index`)
  saturate a large single-purpose repo, so even an identifier-aligned query buries
  the specific target below dozens of equally-lexical chunks. We publish these as
  known-miss rather than curating them away.

### What a learned embedder changes — MEASURED (US-1B)

US-1B landed the real candle `BertModel` backend and re-ran *this exact harness*
with `all-MiniLM-L6-v2`. The standing hypothesis was that a learned embedder would
rescue the NL-phrasing known-misses ("avoid reading the whole file into memory" →
`mmap`). The measurement **refutes that for a natural-language model**: vector-only
recall@5 dropped **0.083 → 0.000**, hybrid recall@5 0.458 → 0.417 — MiniLM is
trained on English sentences, not code, so its nearest neighbors are noise for code
queries. The forward-pass is correct (the paraphrase unit test passes); the model
is the wrong kind. The open lever is a **code-trained** embedder (CodeBERT /
nomic-embed-code / jina-code class), not a generic NL one — that is the OQ-3
follow-up. (Still no live third-party tool comparison, by the same zero-network
honesty rule.) See "Embedding backend status (honest)" below for the full result.

## Chunk-cap sweep (US-4)

> A measured sweep of the two GLOBAL chunk caps `MAX_CHUNK_LINES` /
> `MAX_CHUNK_BYTES` (`crates/apohara-indexer/src/chunker.rs:48`/`:58`), applied to
> all four languages via `chunk_file` → `split_module_run`. The ONLY legitimate
> inputs are the recall/MRR from the external bench above and the footprint from
> the soak above. This is a measured nudge, not a search.

**Method.** For each grid point, both consts were edited, `cargo build --release`
re-run, the external recall bench re-run on BOTH slices (ripgrep Rust `82313cf` +
hugo Go `7d1b1fb`), and the tokio footprint soak re-run (cold force-index, peak
RSS via `/usr/bin/time -v`, DB-on-disk via `stat`). Recall columns are the
**hybrid (RRF)** mode (the tool's actual ranking); the ripgrep **BM25 MRR** is
also tracked because it is the most sensitive metric to a boundary shift. The
acceptance rule: a grid point is only eligible if it does **not** regress
recall@5/@10/MRR on EITHER slice AND keeps DB-on-disk + peak RSS within +10% of
the (200/8192) baseline. Measured on a Ryzen 5 3600 / 48 GB box, default
(feature-hash) embedder.

**Baseline (200 / 8192):** ripgrep hybrid r@5=0.227 r@10=0.273 MRR=0.1191
(BM25 MRR 0.1494); hugo hybrid r@5=0.222 r@10=0.222 MRR=0.2389; tokio peak RSS
21 896 KB, DB 40 607 744 B (38.7 MiB), 16 400 chunks. +10% ceilings: RSS
≤ 24 086 KB, DB ≤ 44 668 518 B.

| lines × bytes | rg hybrid r@5 | rg hybrid r@10 | rg hybrid MRR | rg BM25 MRR | hugo hybrid r@5 | hugo hybrid r@10 | hugo hybrid MRR | tokio RSS (KB) | tokio DB (B) | chunks | verdict |
|---------------|---------------|----------------|---------------|-------------|-----------------|------------------|-----------------|----------------|--------------|--------|---------|
| **200 × 8192 (baseline)** | 0.227 | 0.273 | 0.1191 | **0.1494** | 0.222 | 0.222 | 0.2389 | 21 896 | 40 607 744 | 16 400 | — (reference) |
| 150 × 8192 | 0.227 | 0.273 | 0.1191 | **0.1494** | 0.222 | 0.222 | 0.2389 | 21 736 | 40 562 688 | 16 452 | recall TIE, no gain |
| 300 × 8192 | 0.227 | 0.273 | 0.1191 | **0.1494** | 0.222 | 0.222 | 0.2389 | 21 876 | 38 989 824 | 16 383 | recall TIE, no gain |
| 200 × 16384 | 0.227 | 0.273 | 0.1177 | 0.1471 | 0.222 | 0.222 | 0.2389 | 22 140 | 40 607 744 | 16 397 | **MRR REGRESS (rg)** |
| 300 × 16384 | 0.227 | 0.273 | 0.1177 | 0.1471 | 0.222 | 0.222 | 0.2389 | 21 940 | 38 952 960 | 16 353 | **MRR REGRESS (rg)** |
| 400 × 16384 | 0.227 | 0.227 | 0.1168 | 0.1456 | 0.222 | 0.222 | 0.2389 | 22 056 | 38 936 576 | 16 335 | **recall@10 + MRR REGRESS (rg)** |

**Outcome: NO CHANGE — the (200, 8192) default is measured-optimal (OQ-4).**
Reading the table:

- **No grid point beats the baseline on recall.** The two points that do not
  regress (150 × 8192 and 300 × 8192) reproduce the baseline recall byte-for-byte
  on both slices — they tie, they never win. The bytes-cap-raising points
  (16384) all *lose* ripgrep MRR (hybrid 0.1191 → 0.1177, BM25 0.1494 → 0.1471):
  a 16 KiB module chunk merges runs that an 8 KiB cap kept separate, demoting a
  few target lines a rank or two. The 400-line point additionally drops ripgrep
  **hybrid recall@10** (0.273 → 0.227). The hugo (Go) slice is flat across the
  whole grid — its labeled targets are all whole-symbol or short-module, so
  module-cap changes do not move them — so the Rust slice is the binding
  constraint, and it says *do not raise the caps*.
- **Footprint never regresses** at any point (all RSS ≤ 22 140 KB ≤ 24 086 KB
  ceiling; all DB ≤ 40 607 744 B ≤ 44 668 518 B ceiling) — so footprint is not
  the discriminator; **recall is**, and recall says keep the default.

Because no point improves recall and the only non-regressing points merely tie,
the correct outcome per the US-4 acceptance contract (and OQ-4: "no change +
documented" is an accepted result) is to **keep `MAX_CHUNK_LINES=200` and
`MAX_CHUNK_BYTES=8192`**. The doc comments at `chunker.rs:48`/`:58` are updated
from "UNTUNED" to record this validation; the caps themselves are unchanged.

**No re-soak needed.** Amendment B's mandatory re-soak fires only *if a cap
changes*; since no cap changed, the existing soak rows above stay valid as-is.
(The baseline soak was also independently re-measured on this 48 GB box during
the sweep — 21 896 KB / 40 607 744 B / 16 400 chunks — matching the 48 GB soak
row's 21.5 MB / 39 MB / 16 400 chunks within jitter, confirming the soak claim
holds on both boxes.)

**Rollback / re-tune procedure.** The caps are two `usize` consts in one file.
To change them: edit `MAX_CHUNK_LINES` (`chunker.rs:48`) and/or `MAX_CHUNK_BYTES`
(`chunker.rs:58`), then **re-index** any existing database (`reindex` with
`force=true`, or delete `.apohara-codesearch/index.db`). Because chunk ids are
`path:start-end`, a re-index fully regenerates every boundary — there is **no
on-disk migration and no format change**. To revert this US-4 work specifically,
`git checkout -- crates/apohara-indexer/src/chunker.rs` (restores both consts +
doc comments) and re-index. The determinism/property tests
(`chunk_ids_pairwise_distinct`, `chunk_caps_module_remainder`,
`module_split_prefers_blank_line`, `module_split_reindex_stable`,
`rrf_beats_bm25_alone`, plus the synthetic bench twice-run assertion) are the
regression guard for any future cap change.

## Embedding backend status (honest)

The default build ships **no model** and embeds via a deterministic feature-hash
(`feature-hash-v1`). The pluggable `Embedder` trait, the user-supplied local-file
load, and the `meta` refuse-to-mix guard are all real and tested.

**1b landed (US-1B): the `gguf-embed` feature now runs a REAL candle `BertModel`
forward-pass** (attention-masked mean-pool + L2-normalize) over a user-supplied
local safetensors checkpoint — `std::fs` only, no network, no `hf-hub`. A
`cargo tree -e normal` check asserts **zero ML crates in the DEFAULT build**
(candle/tokenizers are optional + `gguf-embed`-gated; enabling the feature *does*
pull them in — that is the opt-in), so the no-model default stays provable. A unit test loads a real `all-MiniLM-L6-v2`
and confirms the forward-pass is correct: paraphrases embed closer than unrelated
text, vectors are L2-normalized, `embed` is deterministic. A missing/invalid model
still falls back to feature-hash + a stderr warning (never panics, never fetches).

**Measured result — a natural-language embedder does NOT lift code search.**
Re-running the synthetic harness with `all-MiniLM-L6-v2` (a sentence-transformer
trained on English NLI/paraphrase pairs, NOT code) made the vector side *worse*,
not better: vector-only recall@5 went **0.083 (feature-hash) → 0.000 (MiniLM)**,
hybrid recall@5 0.458 → 0.417. The forward-pass is correct (the paraphrase test
passes); the model is simply the wrong tool — it maps English sentences, not code
identifiers, so for code queries its nearest neighbors are noise. This is the
honest *disprove* the plan asked for: the lever is not "any learned model" but a
**code-trained embedder** (CodeBERT / nomic-embed-code / jina-code class). The
infrastructure to plug one in now exists and is tested; trying a code embedder is
the OQ-3 follow-up.

## Per-language cap validation (US-O5 / OQ-5)

> US-4 tuned the global chunk caps (`MAX_CHUNK_LINES`/`MAX_CHUNK_BYTES`) on Rust
> (ripgrep) + Go (hugo) only, leaving TS/Python unmeasured (OQ-5). This closes
> that gap with hand-labeled TypeScript + Python slices. Corpus NOT vendored.

**Pinned commits:** TypeScript `6fbce89821d93a5b761581d9ac540455f38e9acb` (Apache-2.0,
`src/` subtree), django `a2348c85fc6c20087935c74cd99340dd4ef2dcdc` (BSD-3).

**At the default caps (200 / 8192):**

| Slice | known-miss | mode | recall@5 | recall@10 | MRR |
|-------|-----------|------|----------|-----------|-----|
| TypeScript (10 q) | 60% | BM25 | 0.400 | 0.600 | 0.3310 |
| | | vector | 0.000 | 0.000 | 0.0000 |
| | | hybrid | 0.400 | 0.400 | 0.2334 |
| django (10 q) | 40% | BM25 | 0.600 | 0.800 | 0.3244 |
| | | vector | 0.000 | 0.000 | 0.0000 |
| | | hybrid | 0.300 | 0.600 | 0.2708 |

**Spot-check at 300 / 16384** (the cap change US-4 found weak on Rust+Go): TS recall
unchanged (BM25 MRR 0.3310→0.3333, marginal), but django **regressed** — BM25
recall@10 0.800→0.700, MRR 0.3244→0.3067; hybrid recall@5 0.300→0.200. A larger
cap does not help TS and actively hurts Python.

**Verdict — OQ-5 resolved: the 200/8192 default is validated across all four
languages.** The per-language picture matches Rust+Go exactly: the feature-hash
vector side is near-noise on real OSS (vector-only recall is 0 on both TS and
django), hybrid never beats BM25-alone, and raising the caps only dilutes chunks
(measurably so on django). No cap change is warranted. The same honest caveat
holds: a learned embedder (1b) is the lever that could change the vector column;
that is what OQ-3 will re-measure, not the caps.
