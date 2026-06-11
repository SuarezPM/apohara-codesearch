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

## CodeSearchNet `{NL query → code}` benchmark (one-off, not in CI, corpus not vendored)

> Recall@5 / MRR over the **standard CodeSearchNet** corpus — a recognized public
> `{NL query → code}` dataset, not an author-curated one — so the fusion claims
> here are falsifiable against prior art, not only against our own labels. This
> **extends** the sections above; it does not replace them, and it PRESERVES the
> same frozen honesty contract: relevance by `(file, target_line)` (not chunk-id),
> a per-slice known-miss rate ≥ 30%, the reported "hybrid worse than best single
> mode" count, and twice-run byte-identical determinism.

**Prior-art reconciliation.** The headline finding of the sections above is that
the hybrid LOSES to BM25-only on every real-OSS corpus with the feature-hash
backend (the "fusion tax"). This CSN slice is added to test that same picture on a
recognized dataset and reports it honestly — including losses — not a curated win.

**Dataset — version / split / provenance (reproducibility).**

| Field | Value |
|-------|-------|
| Dataset | CodeSearchNet corpus (Husain et al. 2019, [arXiv:1909.09436](https://arxiv.org/abs/1909.09436); [github/CodeSearchNet](https://github.com/github/CodeSearchNet#data)) |
| Source mirror | `https://huggingface.co/datasets/code-search-net/code_search_net` — per-language Parquet at `<lang>/test-00000-of-00001.parquet` (the original github/CodeSearchNet S3 bucket now returns 403; HF is the maintained mirror). `scripts/fetch-csn.sh` downloads the Parquet and converts it to JSONL, renaming the HF fields `func_documentation_string`/`func_code_string` → `docstring`/`code` so the loader is unchanged. |
| Split | **test** split only (held-out evaluation split; train/valid are NOT used) |
| Languages | python, go, javascript — JavaScript is materialized as the **typescript** slice (`.ts`) because CodeSearchNet has NO TypeScript split and TS/JS share a parser family; it is the closest public NL→code proxy for our first-class TypeScript support |
| Records per slice | a deterministic file-order prefix of up to 200 usable records (records with an empty docstring or empty code are skipped) — keeps a run tractable AND byte-stable |
| NL query | the first non-empty line of each record's `docstring` (the CSN challenge's summary-line convention) |

**Checksum (record it yourself — do not trust a number you did not compute).** The
fetch helper does NOT bake in a sha256, because the upstream shards can be
re-published and a stale baked-in sum that silently disagrees is worse than one
recomputed and committed deliberately. After fetching, compute and paste the three
sums here so a second machine can verify byte-identical inputs:

```bash
sha256sum "$APOHARA_CSN_ROOT"/{python,go,typescript}.jsonl
```

| File | records | sha256 |
|------|---------|--------|
| `python.jsonl` | 22176 | `45f4b851e412ce749ccfbf76e7986ae25bc35f16d376d6644cd8f0c76eefec1f` |
| `go.jsonl` | 14291 | `7706e7c78e8e2f7878bc991460db9ad8096579c27c8cd886581fa3ed0558963e` |
| `typescript.jsonl` (from javascript) | 6483 | `3fe4efd9bcd9ffc9365fe3a1c4ceddb5f5753e3c91b47a9487918a983ac34b70` |

> Measured 2026-06-06 on the HF Parquet test split (fields converted to
> `{docstring, code}`). The bench reads a deterministic 200-record prefix per slice.

The dataset VERSION (the test split at the source above) is the reproducibility
anchor; the sha256 you record pins the exact bytes you measured.

**Harness:** `crates/apohara-codesearch/examples/bench-codesearchnet.rs` — an
*example* (never packaged by dist), mirroring `bench-external.rs`: env-gated by
`APOHARA_CSN_ROOT`, twice-run byte-identical determinism assertion, `(file,
target_line)` relevance. It reports **FIVE** arms — BM25-only, vector-only,
hybrid-RRF, and **hybrid+MMR** (the diversity re-rank `mmr_rerank` / `MMR_LAMBDA`
applied AFTER fusion — additive signal the plain three arms do not measure), and
**adaptive** (Story 2: per-query fusion weights from `classify_query_weights` /
`resolve_weights` — BM25-heavy on identifier-shaped queries, vector-heavy on
multi-word NL) — plus the "queries where hybrid < best single mode" count, per
slice.

> **Adaptive arm — recovery gate (AC4): MEASURED, and it does NOT clear the bar
> on CSN (negative result, recorded honestly).** The bar was `adaptive recall@5 >=
> BM25-only` AND `>= plain-hybrid`. On all three CSN slices adaptive falls FAR
> short (python 0.355 vs BM25 0.955; go 0.005 vs 0.860; ts 0.155 vs 0.740). This
> is the *expected* outcome, not a regression, and it confirms the documented
> limitation: CodeSearchNet queries are natural-language docstrings, so
> `classify_query_weights` tags them "vector-heavy" and up-weights the vector arm
> — which with the default feature-hash embedder is near-noise (vector-only:
> 0.340 / 0.005 / 0.035). Adaptive therefore *amplifies* the weak arm on exactly
> the query shape where it cannot help. The recovery adaptive was designed for —
> lifting identifier/symbol queries that plain RRF demotes — is a different query
> shape than CSN's NL docstrings, and the lift it needs is gated on a real
> code-trained embedder (roadmap), not the feature-hash default. **This is why
> `adaptive` ships opt-in and OFF by default.** Net: on this corpus, **BM25-only
> is the configuration to use**; fusion (and adaptive) are a tax with the
> feature-hash backend, published here rather than hidden.

**Dataset fetch (deliberate, standalone).** `scripts/fetch-csn.sh` downloads the
test-split shards and writes `python.jsonl` / `go.jsonl` / `typescript.jsonl`
under `$APOHARA_CSN_ROOT`. It is a plain shell script **NOT invocable by any cargo
target** (no `build.rs`, no `[[example]]`, no test references it), so no `cargo`
command can trigger a network fetch — the "zero network in any cargo command"
contract holds.

```bash
bash scripts/fetch-csn.sh /path/to/csn
APOHARA_CSN_ROOT=/path/to/csn cargo run --release --example bench-codesearchnet
```

Unset `APOHARA_CSN_ROOT` → the harness prints guidance and exits 0 (no network, no
hard dependency). Default (feature-hash) embedder, release binary.

**Results (per slice — python, go, typescript first-class).** Run the harness
against the fetched corpus and paste the four-arm table for each slice below. The
per-slice known-miss rate (queries the hybrid ranks outside top-k) must read ≥ 30%
— if a slice reports below the floor, that is a labeling/loader signal to
investigate, not a number to massage.

Measured 2026-06-06, default feature-hash embedder, 200-record prefix per slice:

| Slice | known-miss | mode | recall@5 | recall@10 | MRR |
|-------|-----------|------|----------|-----------|-----|
| **python** (200 q) | 2 (1%) | BM25-only | **0.955** | 0.990 | 0.8915 |
| | | vector-only | 0.340 | 0.340 | 0.3233 |
| | | hybrid (RRF) | 0.930 | 0.960 | 0.8720 |
| | | hybrid+MMR | 0.880 | 0.920 | 0.8482 |
| | | adaptive | 0.355 | 0.365 | 0.3974 |
| **go** (200 q) | 14 (7%) | BM25-only | **0.860** | 0.930 | 0.7259 |
| | | vector-only | 0.005 | 0.005 | 0.0050 |
| | | hybrid (RRF) | 0.800 | 0.860 | 0.6868 |
| | | hybrid+MMR | 0.735 | 0.835 | 0.6669 |
| | | adaptive | 0.005 | 0.005 | 0.0839 |
| **typescript** (200 q) | 32 (16%) | BM25-only | **0.740** | 0.840 | 0.5703 |
| | | vector-only | 0.035 | 0.035 | 0.0250 |
| | | hybrid (RRF) | 0.675 | 0.745 | 0.5059 |
| | | hybrid+MMR | 0.520 | 0.705 | 0.4664 |
| | | adaptive | 0.155 | 0.165 | 0.1718 |

**"Hybrid worse than best single mode" count (per slice):** python **27**, go
**62**, typescript **85**. As the prior-art sections predicted, hybrid does NOT
beat BM25-alone with the feature-hash backend — the fusion tax is real and grows
as the slice gets lexically harder (python → ts). BM25-only is the strongest arm
on every slice.

> **On the < 30% known-miss floor.** The ≥30% floor is a contract for our
> *author-curated* corpora (where known-miss queries are deliberately committed to
> prevent a self-flattering set). CSN is the *standard external dataset taken as-is*
> — its known-miss here is **measured, not injected**: CSN docstrings are lexically
> close to their code, so BM25 finds most of them and the miss rate is naturally
> low (1–16%). That is a property of the dataset, not a massaged number — and the
> honest takeaway it produces (BM25 wins, fusion is a tax) is the opposite of
> self-flattering.

**Determinism:** metrics are byte-stable. The harness measures each slice twice and
asserts the two metric sets identical before printing; two *separate* process
invocations also produce byte-identical stdout (verifiable with `sha256sum`),
because the feature-hash embedder is deterministic and the RRF tie-break is total —
there is no RNG / seed in this path.

> The tables above are MEASURED (2026-06-06), not placeholders. The corpus is
> still never vendored — anyone can reproduce by running `scripts/fetch-csn.sh`
> (HF Parquet → JSONL) and the bench; the sha256s above pin the exact bytes
> measured. The harness, the five arms, the determinism gate, and the metric unit
> tests (`cargo test -p apohara-codesearch --example bench-codesearchnet`) all
> ship and pass with the corpus absent.

## v0.8 — CodeSearchNet with a REAL learned embedder (EmbeddingGemma)

> **What changed vs every section above.** All prior CSN/real-OSS numbers were
> measured with the default **feature-hash** embedder, whose headline finding is
> the *fusion tax*: hybrid LOSES to BM25-only and the vector arm is near-noise
> (vector-only recall@5 of 0.340 / 0.005 / 0.035 on python / go / typescript).
> v0.8 re-runs the SAME harness, unchanged, with the REAL **EmbeddingGemma**
> backend (`embedder_gemma.rs`, pure-candle Gemma3 256-d Matryoshka, asymmetric
> query/document prompts, opt-in behind `--features gguf-embed` +
> `APOHARA_EMBED_MODEL`). The result **inverts the fusion tax**: the vector arm
> becomes a first-class retriever and **hybrid beats BM25-only on every NL
> slice**, and the AC4 recovery gate (G2) **clears on every identifier slice**.

**Backend & provenance.**

| Field | Value |
|-------|-------|
| Embedder | EmbeddingGemma (`embeddinggemma-300m`), pure-candle Gemma3 forward pass, 256-d Matryoshka output, L2-normalized, asymmetric prompts (`embed_query` prepends the query instruction, `embed_document` the document prefix). Parity vs the reference oracle: cosine 0.999983 (S3). |
| Activation | `APOHARA_EMBED_MODEL=/path/to/eg-st cargo run --release --features gguf-embed --example bench-codesearchnet` (and `--example bench-csn-identifier`). The default build ships NO model and the `cargo tree -e normal -p apohara-indexer` candle scan stays 0 — the ML deps are example-gated, not in the default binary. |
| Index/query path | UNCHANGED from server.rs: chunks are indexed via `storage::index → embed_document`, queries via `vector_query_with → knn_query_with → embed_query`. The bench already routed through `vector_query_with`, so the query arm uses `embed_query` (asymmetric prompt) with no harness change required. |
| Device | CPU (Ryzen 5 3600). The 300M model on CPU is heavy: each NL slice (200 records × 5 arms × 2 determinism passes) takes minutes — this is a one-off, not CI. |
| Dataset | identical to the feature-hash CSN section above (same HF test-split Parquet → JSONL, same sha256s); the ONLY variable changed is the embedder. |

### NL `{docstring → code}` slices — EmbeddingGemma, 200-record prefix

Measured 2026-06-06, `APOHARA_EMBED_MODEL=/tmp/eg-st`, release binary. Same
`(file, target_line)` relevance, same twice-run byte-identical determinism gate.
The **Δ** column is EmbeddingGemma recall@5 minus the feature-hash baseline from
the section above.

| Slice | known-miss | mode | recall@5 | recall@10 | MRR | Δ recall@5 vs feature-hash |
|-------|-----------|------|----------|-----------|-----|----------------------------|
| **python** (200 q) | 1 (0%) | BM25-only | 0.955 | 0.990 | 0.8915 | — (BM25 unchanged) |
| | | vector-only | 0.950 | 0.975 | 0.8939 | **+0.610** (0.340 → 0.950) |
| | | hybrid (RRF) | **0.980** | 0.985 | 0.9322 | +0.050 (0.930 → 0.980) |
| | | hybrid+MMR | 0.980 | 0.985 | 0.9246 | +0.100 |
| | | adaptive | 0.975 | 0.980 | 0.9335 | +0.620 |
| **go** (200 q) | 2 (1%) | BM25-only | 0.860 | 0.930 | 0.7259 | — |
| | | vector-only | **0.990** | 0.990 | 0.9092 | **+0.985** (0.005 → 0.990) |
| | | hybrid (RRF) | **0.970** | 0.990 | 0.8522 | +0.170 (0.800 → 0.970) |
| | | hybrid+MMR | 0.945 | 0.980 | 0.8404 | +0.210 |
| | | adaptive | 0.985 | 0.990 | 0.8875 | +0.980 |
| **typescript** (200 q) | 9 (4%) | BM25-only | 0.740 | 0.840 | 0.5703 | — |
| | | vector-only | **0.885** | 0.935 | 0.7909 | **+0.850** (0.035 → 0.885) |
| | | hybrid (RRF) | **0.920** | 0.945 | 0.7521 | +0.245 (0.675 → 0.920) |
| | | hybrid+MMR | 0.860 | 0.910 | 0.7316 | +0.340 |
| | | adaptive | 0.915 | 0.935 | 0.7854 | +0.760 |

**Headline (NL): the fusion tax is GONE with a real embedder.**
- The **vector arm is now first-class**: it MATCHES BM25 on python (0.950 vs
  0.955) and BEATS it outright on go (0.990 vs 0.860) and typescript (0.885 vs
  0.740) — the exact opposite of the feature-hash near-noise (0.340 / 0.005 /
  0.035).
- **Hybrid (RRF) beats BM25-only on ALL THREE slices** (python 0.980 > 0.955,
  go 0.970 > 0.860, ts 0.920 > 0.740). Under feature-hash hybrid LOST on every
  slice; with EmbeddingGemma fusion is a *gain*, confirming the v0.7 hypothesis
  (S1) that a learned embedder is the lever that flips the picture.
- "Hybrid worse than best single mode" counts drop in spirit but stay non-zero
  (python 13, go 35, ts 47) — fusion is now net-positive on aggregate recall yet
  still demotes *some* individual queries; honest, not a clean sweep.
- Adaptive tags multi-word NL docstrings vector-heavy, which now HELPS (the
  vector arm is strong): adaptive tracks hybrid closely (0.975 / 0.985 / 0.915).
  This is the inverse of the feature-hash CSN result, where vector-heavy
  amplified a noise arm and adaptive collapsed.

### Identifier `{func_name → code}` slice — the AC4 recovery gate (G2)

The AC4 gate needs **identifier-shaped** queries, not NL docstrings — the query
shape `classify_query_weights` up-weights BM25 for. A sibling harness,
`crates/apohara-codesearch/examples/bench-csn-identifier.rs`, uses the CSN
`func_name` (the function IDENTIFIER, e.g. `get_vid_from_url`,
`YouTube.parse_url`) as the query against the SAME `code` body. The corpus is the
`func_name`/`func_code_string` columns of the same HF test-split Parquet,
extracted to `$APOHARA_CSN_IDENT_ROOT/{python,go,typescript}.jsonl` (func_name in
the `docstring` slot so one loader serves both benches), func_names filtered
non-empty. The query is embedded through the SAME `embed_query` path server.rs
runs (asymmetric prompt kept — AC4 asks whether the system *as deployed* recovers
on identifier queries, so the bench mirrors the deployed path rather than
stripping the prompt).

**AC4 (G2) bar:** `adaptive recall@5 >= BM25-only AND adaptive recall@5 >= plain-hybrid`.

Identifier corpus (record it yourself):

| File | records | sha256 |
|------|---------|--------|
| `python.jsonl` | 22176 | `aa9d09d67b5045bc87bede96560b6a7cb085de74f47ab3cf86c743cd287af573` |
| `go.jsonl` | 14291 | `1bbd747d3b5c6e221d4a2672bf8942773277575de983c659895ea5c8307cbd6b` |
| `typescript.jsonl` (from javascript) | 4441 | `554eb678900c1c27a2c074425fd85a8d60d25ac63730b344c8321ddcae6f3cbc` |

Measured 2026-06-06, `APOHARA_EMBED_MODEL=/tmp/eg-st`, **100-record prefix** per
slice (the identifier bench caps at 100 — the 300M model on CPU is heavy; bump
`MAX_RECORDS_PER_SLICE` for a deeper run):

| Slice | mode | recall@5 | recall@10 | MRR |
|-------|------|----------|-----------|-----|
| **python** (100 q) | BM25-only | 0.980 | 0.980 | 0.7552 |
| | vector-only | 0.960 | 0.980 | 0.9094 |
| | hybrid (RRF) | 0.980 | 0.990 | 0.8548 |
| | hybrid+MMR | 0.990 | 0.990 | 0.8480 |
| | **adaptive** | **0.980** | 0.980 | 0.8118 |
| **go** (100 q) | BM25-only | 0.980 | 1.000 | 0.8779 |
| | vector-only | 0.970 | 0.990 | 0.8859 |
| | hybrid (RRF) | 0.990 | 1.000 | 0.9153 |
| | hybrid+MMR | 1.000 | 1.000 | 0.9103 |
| | **adaptive** | **0.990** | 1.000 | 0.9136 |
| **typescript** (100 q) | BM25-only | 0.990 | 1.000 | 0.8568 |
| | vector-only | 0.980 | 0.980 | 0.9025 |
| | hybrid (RRF) | 1.000 | 1.000 | 0.9217 |
| | hybrid+MMR | 1.000 | 1.000 | 0.9062 |
| | **adaptive** | **1.000** | 1.000 | 0.9100 |

**AC4 (G2) verdict — GO on all three slices:**

| Slice | adaptive r@5 | ≥ BM25 r@5 | ≥ hybrid r@5 | AC4 |
|-------|-------------|------------|--------------|-----|
| python | 0.980 | 0.980 ✓ | 0.980 ✓ | **GO** (exact tie) |
| go | 0.990 | 0.980 ✓ | 0.990 ✓ | **GO** |
| typescript | 1.000 | 0.990 ✓ | 1.000 ✓ | **GO** |

> **Read the GO honestly — this is a NO-REGRESSION result on a saturated task,
> not a dramatic win.** The identifier task is lexically EASY by construction: each
> query is a `func_name` that appears verbatim in its own definition, over a corpus
> of isolated one-function files, so every arm saturates near 1.0 and the margins
> are 0–1 point (python is an exact three-way tie at 0.980). What AC4 verifies here
> is therefore that adaptive fusion **does not drag the exact-symbol lookup below
> BM25** — the precise failure feature-hash exhibited (below) — NOT that it lifts it
> dramatically. The lift adaptive *can* deliver shows up on the NL slices above
> (where hybrid/adaptive beat BM25 by a real margin); on identifier lookups the bar
> is "don't regress", and with EmbeddingGemma it is cleared, whereas feature-hash
> regressed it.

> **Why this is the real test, and why feature-hash failed it.** Re-running the
> identifier bench with the DEFAULT feature-hash embedder gives **NO-GO** on all
> three slices: adaptive recall@5 0.970 / 0.950 / 0.980 sits 1–3 points BELOW
> BM25-only (0.980 / 0.980 / 0.990), because the vector arm is near-noise
> (vector-only recall@5 0.160 / 0.030 / 0.010) and even a BM25-heavy adaptive
> fusion can only drag itself down by mixing it in. With EmbeddingGemma the
> vector arm jumps to 0.960 / 0.970 / 0.980, so adaptive fusion has a real second
> signal to fuse and reaches BM25 parity or better on every slice — **AC4
> clears.** The recovery adaptive was designed for is real, and it is gated on a
> real code-capable embedder exactly as the v0.7 limitation predicted. This is
> the empirical basis for Decision **G2**.

**Determinism:** both benches measure each slice twice and assert byte-identical
metrics before printing (the EmbeddingGemma forward pass is deterministic on CPU
and the RRF tie-break is total — no RNG in this path). Metric unit tests ship and
pass with the corpus absent:
`cargo test -p apohara-codesearch --features gguf-embed --example bench-csn-identifier`.

> **Honesty note.** The NL slices use a 200-record prefix and the identifier
> slices a 100-record prefix (the identifier bench's lower cap reflects the CPU
> cost of the 300M model — documented, not hidden). Both are deterministic
> file-order prefixes, so the numbers are byte-stable and reproducible; a deeper
> run only needs a larger cap and more wall-clock. The corpus is never vendored;
> the sha256s pin the exact bytes measured.

## v0.3.0 baseline (corpus A frozen at `d045e86c`)

Per the v0.3.0 plan (`.omc/plans/apohara-codesearch-3frentes.md` §6 "Corpus freeze"), the v0.3.0 default-flip measurement uses a frozen copy of `examples/bench-corpus/` rather than the live corpus, so the measurement is reproducible independent of subsequent grammar-label regeneration work.

- **Corpus A (BENCHMARK baseline):** `tests/fixtures/bench-corpus-frozen-A/`, content-hash `d045e86ca978935f1a292b631941b8bde3d3341f49179a94fdfbebb1a2890b29`, freeze commit `f52bdfd`. 22 files, byte-identical to `examples/bench-corpus/` at the v0.2.0 release.
- **Corpus B (golden test):** `tests/fixtures/bench-corpus-frozen-B/queries.json`, content-hash `f5da3d598daee07528676a4ab528db7a70c15a7bd37d245da443857c55338ab2`, freeze commit `15a2deb`. 10 hand-picked queries for the F3 default-flip golden test.
- **Guard test:** `crates/apohara-codesearch/tests/corpus_freeze.rs` pins both content-hashes and fails on any drift. Re-freeze by updating `CORPUS_A_EXPECTED_HASH` / `CORPUS_B_EXPECTED_HASH` in the test and adding a `chore(bench): refreeze corpus X` commit.

The F3-MEASURE story will populate this section with the v0.2.0-baseline recall/MRR numbers and the post-flip delta. Until then, this section documents the freeze only.

## v0.3.0 flip measurement (F3-MEASURE, 2026-06-11)

Run on **Corpus A** (frozen at v0.2.0, content-hash `d045e86c...`).
Tool: `cargo run --release --example bench-search` (which exercises the
indexer directly via `bm25_query` + `vector_query` + `rrf_fuse`).

### Baseline (v0.2.0 defaults)

```
queries: 24 total, 9 known-miss (38%)

| Mode         | recall@5 | recall@10 |   MRR  |
|--------------|----------|-----------|--------|
| BM25-only    |  0.542   |   0.625   | 0.3258 |
| vector-only  |  0.083   |   0.083   | 0.0625 |
| hybrid (RRF) |  0.458   |   0.542   | 0.2852 |

queries where hybrid < best single mode: 9
```

Interpretation: BM25 wins on this small synthetic corpus. The vector arm is
near-noise (feature-hash embedder, no learned model). Hybrid is slightly
worse than BM25-alone, matching the historical v0.5 / v0.7 measurements.

### v0.3.0 proposed defaults (NOT measured in this bench)

`adaptive=true` and `diversify=true` live in the **server-side** `search_code`
wrapper (`crates/apohara-codesearch/src/server.rs`), not in the indexer-level
`rrf_fuse`. The bench-search harness calls `rrf_fuse` directly and therefore
CANNOT measure the proposed v0.3.0 defaults from the indexer surface.

**The F3 split criteria (per .omc/plans/apohara-codesearch-3frentes.md §6)
therefore need to be applied against:**

- The `adaptive` heuristic: OQ-3 v0.5 already DISPROVED a generic NL embedder
  for code search; the adaptive heuristic is lexical-only (no corpus
  signal) and was never measured against CodeSearchNet identifier slice
  (deferred per OQ-3 follow-up). The criterion is data-driven; we do not
  have the data here.
- The `diversify` (MMR) post-process: OQ-3 v0.5 was UNTESTED. The criterion
  is "raises recall@10 OR drops mean pairwise cos-sim of top-3 by ≥15%".
  We cannot measure it from the indexer surface.

### Recommended decision

Given the measurement gap above, F3-FLIP-CHECK CANNOT satisfy the
positive-lift criterion for `diversify` (we cannot measure it). The
data-driven flips are DEFERRED to v0.4.0 where we will:

1. Add a real embedder measurement on the CodeSearchNet identifier slice
   (deferred since v0.5 OQ-3).
2. Add an MMR post-process measurement to the bench harness (server-side
   surfacing, requires plumbing `search_code` results into the bench).

For v0.3.0: ship the 5 new grammars + the corpus freeze work + the
OpenSSF Scorecard audit + the cargo-audit pin, but **do NOT flip the
adaptive/diversify defaults**. The CHANGELOG entry should make the
deferral explicit so users know v0.3.0 is structural-extraction-focused,
not ranking-focused.

If Pablo disagrees and wants the flips on the basis of "the criteria are
untestable from here, but the flip is low-risk and the rollback path is
documented in the plan (§10)" — proceed with both flips, document the
gap, ship the rollback plan in CHANGELOG.
