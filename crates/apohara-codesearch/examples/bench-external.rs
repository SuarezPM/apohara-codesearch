// SPDX-License-Identifier: MIT OR Apache-2.0

//! External real-OSS retrieval benchmark (one-off, NOT in CI, corpus NOT vendored).
//!
//! Mirrors `examples/bench-search.rs` but points `index_repo` at a REAL
//! third-party repository on local disk instead of the checked-in synthetic
//! corpus. Two slices are measured independently:
//!
//! - a `ripgrep` (Rust) slice — ~22 hand-labeled developer intents, the primary
//!   single-language recall signal;
//! - a `hugo` (Go) slice — a non-Rust slice (≥8 labels) so a later chunk-cap
//!   sweep (US-4) has a Go data point on the SAME global caps, not a Rust-only one.
//!
//! ## Why an env-pointed corpus (and never vendored)
//!
//! The repos are cloned ONCE, manually, outside this repo (a one-time online
//! prereq, NOT part of any build/test/runtime). The harness reads them only from
//! local disk via `APOHARA_BENCH_EXTERNAL_ROOT`, which must point at the parent
//! directory holding `ripgrep/` and `hugo/` checkouts. Nothing here is committed
//! under `examples/bench-corpus/`, so the SACRED "zero network, corpus not
//! vendored" contract holds. If the env var is unset, the harness prints how to
//! set it and exits 0 (so `cargo run --example` never hard-fails a clean tree).
//!
//! ## Honesty contract (identical to bench-search.rs)
//!
//! Relevance is `(file, target_line)`, NOT a `path:start-end` chunk id: a hit
//! COUNTS as relevant when its repo-relative `file_path` equals the label's
//! `file` AND its hydrated `[start_line, end_line]` contains `target_line`. So a
//! label survives any chunk-boundary change (this is what lets US-4 sweep the
//! caps without invalidating the labels). At least 30% of EACH slice's labels are
//! committed KNOWN-MISS cases (`known_miss = true`): the relevant target is
//! outside the hybrid top-k, or found by only one mode. Publishing the tool's own
//! failures is the guard against a self-flattering corpus.
//!
//! ## Determinism
//!
//! Each slice's metrics are measured twice and asserted byte-identical before
//! printing, exactly as bench-search.rs does — the feature-hash embedder is
//! deterministic and the RRF tie-break is total.
//!
//! Run: `APOHARA_BENCH_EXTERNAL_ROOT=/path/to/apohara-soak \
//!       cargo run --release --example bench-external`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apohara_indexer::{
    bm25_query, hydrate, index_repo, migrate, open_db, rrf_fuse, vector_query, HydratedHit,
};

/// Env var pointing at the parent directory that holds the pre-cloned `ripgrep/`
/// and `hugo/` checkouts (the one-time manual clone, never vendored here).
const EXTERNAL_ROOT_ENV: &str = "APOHARA_BENCH_EXTERNAL_ROOT";

/// How deep each mode's ranked list is fetched. `recall@5`/`recall@10` are read
/// off the same length-`K` list, so `K` must be >= the largest reported cutoff.
const K: usize = 10;

/// The two recall cutoffs reported per mode.
const CUTOFFS: [usize; 2] = [5, 10];

/// One labeled query. Identical shape to bench-search.rs's `Label`.
///
/// Relevance is `(file, target_line)`: a hit is relevant iff `hit.file_path`
/// equals `file` (repo-relative, forward-slashed) and
/// `hit.start_line <= target_line <= hit.end_line`. `symbol` names the human
/// target (documentation + traceability); `target_line` is what the rule
/// matches. `known_miss` marks an intentionally hard case the hybrid ranks
/// outside top-k or that only one mode finds.
struct Label {
    query: &'static str,
    file: &'static str,
    // `symbol` and `note` are committed documentation only (not read at run
    // time); the `#[allow(dead_code)]` mirrors bench-search.rs so `-D warnings`
    // stays happy.
    #[allow(dead_code)]
    symbol: &'static str,
    target_line: i64,
    known_miss: bool,
    #[allow(dead_code)]
    note: &'static str,
}

/// One measured corpus slice: a sub-directory under `EXTERNAL_ROOT_ENV` plus its
/// label table. The `name` is the report heading; `subdir` is appended to the
/// external root to locate the on-disk checkout.
struct Slice {
    name: &'static str,
    subdir: &'static str,
    labels: &'static [Label],
}

/// ripgrep (Rust) slice — pinned SHA 82313cf (see BENCHMARK.md). Targets were
/// read off the live source on disk; lines sit INSIDE the target function body
/// so any reasonable chunk boundary contains them.
///
/// `known_miss` is set from the MEASURED outcome at SHA 82313cf, not a guess: a
/// label is a known-miss when the relevant chunk is NOT in the hybrid top-10, OR
/// is found by only one mode. On a 52k-LOC repo the feature-hash backend misses
/// a LOT — common tokens (`search`, `path`, `match`, `glob`) bury the specific
/// target, and the deterministic vector list (1/22 hits) cannot rescue it; RRF
/// then often demotes a BM25 win out of the top-10. That is the honest signal.
///
/// 16 of 22 are known-miss (73%), far above the 30% floor — published failures.
const RIPGREP_LABELS: &[Label] = &[
    // ---- Tool-wins: relevant target in the HYBRID top-10 (measured). ----
    Label {
        query: "memory map open a file as mmap",
        file: "crates/searcher/src/searcher/mmap.rs",
        symbol: "MmapChoice::open",
        target_line: 65,
        known_miss: false,
        note: "mmap + open + Mmap::map align; bm=1, hy=1 (best win on the slice)",
    },
    Label {
        query: "default color specs for path line and match",
        file: "crates/printer/src/color.rs",
        symbol: "default_color_specs",
        target_line: 10,
        known_miss: false,
        note: "default_color_specs named directly; bm=2, hy=5",
    },
    Label {
        query: "parse human readable size with k m g suffix",
        file: "crates/cli/src/human.rs",
        symbol: "parse_human_readable_size",
        target_line: 79,
        known_miss: false,
        note: "parse_human_readable_size named directly; bm=3, hy=5",
    },
    Label {
        query: "inner literals extraction from regex for prefilter",
        file: "crates/regex/src/literal.rs",
        symbol: "InnerLiterals::new",
        target_line: 54,
        known_miss: false,
        note: "InnerLiterals + literal align; bm=2, hy=3",
    },
    Label {
        query: "search a path with a matcher and write color output",
        file: "crates/core/search.rs",
        symbol: "search_path",
        target_line: 380,
        known_miss: false,
        note: "search_path + Matcher + WriteColor align; bm=4, hy=7",
    },
    Label {
        query: "json serialize a match message for output",
        file: "crates/printer/src/jsont.rs",
        symbol: "Match serialize",
        target_line: 94,
        known_miss: false,
        note: "the ONE query the vector mode also finds; bm=4, ve=1, hy=2",
    },
    // ---- Known-miss (measured): relevant target OUT of hybrid top-10, or  ----
    // ---- found by only one mode. Identifier-aligned phrasings that still  ----
    // ---- miss because common tokens bury the target in a 52k-LOC repo.    ----
    Label {
        query: "gitignore matched path against patterns",
        file: "crates/ignore/src/gitignore.rs",
        symbol: "Gitignore::matched",
        target_line: 191,
        known_miss: true,
        note: "all modes miss: `matched`/`path`/`patterns` are too common across the repo",
    },
    Label {
        query: "parse command line arguments into low level args",
        file: "crates/core/flags/parse.rs",
        symbol: "parse_low",
        target_line: 64,
        known_miss: true,
        note: "BM25-only finds it (bm=6); hybrid demotes it to rank 11, out of top-10",
    },
    Label {
        query: "build parallel directory walker iterator",
        file: "crates/ignore/src/walk.rs",
        symbol: "WalkBuilder::build_parallel",
        target_line: 625,
        known_miss: true,
        note: "all modes miss: `build_parallel` is outranked by other walk.rs chunks",
    },
    Label {
        query: "glob is match against a path",
        file: "crates/globset/src/glob.rs",
        symbol: "GlobMatcher::is_match",
        target_line: 142,
        known_miss: true,
        note: "all modes miss: `is_match`/`glob`/`path` saturate globset",
    },
    Label {
        query: "decompression command for a matching file glob",
        file: "crates/cli/src/decompress.rs",
        symbol: "DecompressionMatcher::command",
        target_line: 179,
        known_miss: true,
        note: "all modes miss: `command`/`glob`/`matching` outrank the target method",
    },
    Label {
        query: "counter writer counts bytes written",
        file: "crates/printer/src/counter.rs",
        symbol: "CounterWriter::count",
        target_line: 24,
        known_miss: true,
        note: "all modes miss: the one-line `count` accessor is buried under longer chunks",
    },
    Label {
        query: "matched stripped path against gitignore globs",
        file: "crates/ignore/src/gitignore.rs",
        symbol: "Gitignore::matched_stripped",
        target_line: 248,
        known_miss: true,
        note: "BM25-only finds it (bm=7); hybrid demotes it to rank 13, out of top-10",
    },
    Label {
        query: "searcher search file by path using mmap or buffer",
        file: "crates/searcher/src/searcher/mod.rs",
        symbol: "Searcher::search_path",
        target_line: 643,
        known_miss: true,
        note: "all modes miss: `search`/`path`/`file` saturate the searcher crate",
    },
    // ---- Known-miss: natural-language phrasings whose vocabulary does NOT  ----
    // ---- overlap the code identifiers, so BOTH modes miss the target.      ----
    Label {
        query: "skip files the user asked version control to ignore",
        file: "crates/ignore/src/gitignore.rs",
        symbol: "Gitignore::matched",
        target_line: 191,
        known_miss: true,
        note: "NL paraphrase of gitignore: no 'gitignore'/'matched' token — both miss",
    },
    Label {
        query: "walk a folder tree across several threads at once",
        file: "crates/ignore/src/walk.rs",
        symbol: "WalkBuilder::build_parallel",
        target_line: 625,
        known_miss: true,
        note: "describes parallel walk without 'parallel'/'WalkParallel' — both miss",
    },
    Label {
        query: "turn a count of bytes into a friendly kilobyte string",
        file: "crates/cli/src/human.rs",
        symbol: "parse_human_readable_size",
        target_line: 79,
        known_miss: true,
        note: "NL phrasing of human-readable size — both modes miss",
    },
    Label {
        query: "avoid reading the whole file into memory",
        file: "crates/searcher/src/searcher/mmap.rs",
        symbol: "MmapChoice::open",
        target_line: 65,
        known_miss: true,
        note: "describes mmap by intent, not by name — both modes miss",
    },
    Label {
        query: "highlight matching text in a different colour on screen",
        file: "crates/printer/src/color.rs",
        symbol: "default_color_specs",
        target_line: 10,
        known_miss: true,
        note: "British 'colour' + NL phrasing — both modes miss",
    },
    Label {
        query: "find a short fixed string inside the search pattern",
        file: "crates/regex/src/literal.rs",
        symbol: "InnerLiterals::new",
        target_line: 54,
        known_miss: true,
        note: "describes literal-extraction without 'literal' — both modes miss",
    },
    Label {
        query: "run an external program to unpack a compressed archive",
        file: "crates/cli/src/decompress.rs",
        symbol: "DecompressionMatcher::command",
        target_line: 179,
        known_miss: true,
        note: "NL phrasing of decompression command — both modes miss",
    },
    Label {
        query: "check a filename against a wildcard expression",
        file: "crates/globset/src/glob.rs",
        symbol: "GlobMatcher::is_match",
        target_line: 142,
        known_miss: true,
        note: "'wildcard expression' not 'glob'/'is_match' — both modes miss",
    },
];

/// hugo (Go) slice — pinned SHA 7d1b1fb (see BENCHMARK.md). Same frozen rule;
/// gives US-4 a non-Rust signal on the GLOBAL chunk caps. `known_miss` is again
/// set from the MEASURED outcome at SHA 7d1b1fb (target out of hybrid top-10, or
/// found by only one mode).
///
/// 7 of 9 are known-miss (78%), far above the 30% floor.
const HUGO_LABELS: &[Label] = &[
    // ---- Tool-wins: relevant target in the HYBRID top-10 (measured). ----
    Label {
        query: "new file change batcher with poll interval",
        file: "watcher/batcher.go",
        symbol: "New batcher",
        target_line: 35,
        known_miss: false,
        note: "Batcher + interval + poll align; bm=1, hy=1",
    },
    Label {
        query: "split pages into paginated elements of a given size",
        file: "resources/page/pagination.go",
        symbol: "splitPages",
        target_line: 211,
        known_miss: false,
        note: "splitPages + size align; bm=1, hy=1",
    },
    // ---- Known-miss (measured): out of hybrid top-10, or single-mode-only. ----
    Label {
        query: "minify css and js with the minifier client",
        file: "minifiers/minifiers.go",
        symbol: "Client.Minify",
        target_line: 56,
        known_miss: true,
        note: "all modes miss: `Minify` recurs across the minifiers package",
    },
    Label {
        query: "convert markdown to html with goldmark converter",
        file: "markup/goldmark/convert.go",
        symbol: "goldmarkConverter.Convert",
        target_line: 312,
        known_miss: true,
        note: "BM25-only finds it (bm=6); hybrid demotes it to rank 11, out of top-10",
    },
    Label {
        query: "inverted index search documents by keywords",
        file: "related/inverted_index.go",
        symbol: "InvertedIndex.Search",
        target_line: 346,
        known_miss: true,
        note: "all modes miss: `Search`/`index` saturate the related package",
    },
    Label {
        query: "open a file from the root mapping filesystem",
        file: "hugofs/rootmapping_fs.go",
        symbol: "RootMappingFs.Open",
        target_line: 298,
        known_miss: true,
        note: "BM25-only finds it (bm=9); hybrid demotes it to rank 17, out of top-10",
    },
    // ---- Known-miss: NL phrasings that miss the Go identifiers. ----
    Label {
        query: "shrink stylesheet and script files before serving",
        file: "minifiers/minifiers.go",
        symbol: "Client.Minify",
        target_line: 56,
        known_miss: true,
        note: "NL phrasing of minify — both modes miss",
    },
    Label {
        query: "watch a directory and batch up rapid edits",
        file: "watcher/batcher.go",
        symbol: "New batcher",
        target_line: 35,
        known_miss: true,
        note: "describes batched file-watching without 'batcher' — both modes miss",
    },
    Label {
        query: "break a long list of items across numbered result pages",
        file: "resources/page/pagination.go",
        symbol: "splitPages",
        target_line: 211,
        known_miss: true,
        note: "NL phrasing of pagination — both modes miss",
    },
];

/// The slices measured by this harness, in report order.
const SLICES: &[Slice] = &[
    Slice {
        name: "ripgrep (Rust)",
        subdir: "ripgrep",
        labels: RIPGREP_LABELS,
    },
    Slice {
        name: "hugo (Go)",
        subdir: "hugo",
        labels: HUGO_LABELS,
    },
];

fn main() -> Result<()> {
    let Some(root) = std::env::var_os(EXTERNAL_ROOT_ENV) else {
        // Unset env is the clean-tree default: print guidance and exit 0 so a
        // bare `cargo run --example bench-external` never fails CI-like checks.
        println!(
            "{EXTERNAL_ROOT_ENV} is unset. Point it at the parent directory holding the\n\
             pre-cloned `ripgrep/` and `hugo/` checkouts (the one-time manual clone), e.g.:\n\n  \
             {EXTERNAL_ROOT_ENV}=/home/you/apohara-soak \\\n    \
             cargo run --release --example bench-external\n\n\
             The corpus is read ONLY from local disk and is never vendored into this repo."
        );
        return Ok(());
    };
    let root = PathBuf::from(root);

    for slice in SLICES {
        let corpus = root.join(slice.subdir);
        if !corpus.is_dir() {
            anyhow::bail!(
                "slice '{}' corpus not found at {} — clone it there first (one-time manual prereq)",
                slice.name,
                corpus.display()
            );
        }

        // Determinism gate: measure twice, require byte-identical metrics before
        // printing — same contract as bench-search.rs.
        let first = measure(&corpus, slice.labels)
            .with_context(|| format!("first measurement pass for slice '{}'", slice.name))?;
        let second = measure(&corpus, slice.labels)
            .with_context(|| format!("second measurement pass for slice '{}'", slice.name))?;
        assert_eq!(
            first, second,
            "non-deterministic metrics for slice '{}' — recall/MRR must be byte-stable",
            slice.name
        );

        print_report(slice.name, &first);
        println!();
    }

    Ok(())
}

/// Per-mode aggregate metrics. Equality drives the determinism gate.
#[derive(Debug, Clone, PartialEq)]
struct ModeMetrics {
    /// `recall@5`, `recall@10` in the order of [`CUTOFFS`].
    recall: [f64; CUTOFFS.len()],
    /// Mean reciprocal rank over all labeled queries (0 contribution on a miss).
    mrr: f64,
}

/// The full measurement for one slice: metrics for each mode plus the count of
/// queries where the hybrid's best relevant rank is strictly worse than the best
/// single mode's.
#[derive(Debug, Clone, PartialEq)]
struct Measurement {
    bm25: ModeMetrics,
    vector: ModeMetrics,
    hybrid: ModeMetrics,
    hybrid_worse_than_best_single: usize,
    total_queries: usize,
    known_miss_queries: usize,
}

/// Run all three modes for every label in `labels` against `corpus` and
/// aggregate the metrics. Mirrors bench-search.rs::measure exactly, only the
/// label table and corpus path are parameters here.
fn measure(corpus: &Path, labels: &[Label]) -> Result<Measurement> {
    // Index into a temp DB OUTSIDE the corpus so the corpus tree is never
    // polluted and the walker never sees the database file.
    let tmp = std::env::temp_dir().join(format!(
        "apohara-bench-ext-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&tmp).context("create temp index dir")?;
    let db_path = tmp.join("bench-index.db");

    let result = (|| -> Result<Measurement> {
        let conn = open_db(&db_path).context("open temp index db")?;
        migrate(&conn).context("migrate temp index db")?;
        index_repo(&conn, corpus).context("index external corpus")?;

        let mut bm_ranks = Vec::with_capacity(labels.len());
        let mut ve_ranks = Vec::with_capacity(labels.len());
        let mut hy_ranks = Vec::with_capacity(labels.len());

        // Smallest 1-based rank at which a relevant hit appears in `ids`, or
        // `None`. Relevance is the frozen line-region rule (same file, range
        // contains target line) — robust to chunk-boundary changes.
        let relevant_rank = |ids: &[String], label: &Label| -> Result<Option<usize>> {
            for (i, id) in ids.iter().enumerate() {
                if let Some(hit) = hydrate(&conn, id).context("hydrate hit")? {
                    if is_relevant(&hit, label) {
                        return Ok(Some(i + 1));
                    }
                }
            }
            Ok(None)
        };

        for label in labels {
            let bm = bm25_query(&conn, label.query, K).context("bm25_query")?;
            let ve = vector_query(&conn, label.query, K).context("vector_query")?;
            let hy = rrf_fuse(&bm, &ve, apohara_indexer::RRF_K);

            let bm_ids: Vec<String> = bm.into_iter().map(|(id, _)| id).collect();
            let ve_ids: Vec<String> = ve.into_iter().map(|h| h.chunk_id).collect();
            let hy_ids: Vec<String> = hy.into_iter().map(|(id, _)| id).collect();

            bm_ranks.push(relevant_rank(&bm_ids, label)?);
            ve_ranks.push(relevant_rank(&ve_ids, label)?);
            hy_ranks.push(relevant_rank(&hy_ids, label)?);
        }

        let hybrid_worse = (0..labels.len())
            .filter(|&i| {
                let best_single = min_rank(bm_ranks[i], ve_ranks[i]);
                worse_than(hy_ranks[i], best_single)
            })
            .count();

        Ok(Measurement {
            bm25: aggregate(&bm_ranks),
            vector: aggregate(&ve_ranks),
            hybrid: aggregate(&hy_ranks),
            hybrid_worse_than_best_single: hybrid_worse,
            total_queries: labels.len(),
            known_miss_queries: labels.iter().filter(|l| l.known_miss).count(),
        })
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// The frozen relevance predicate: same file AND the hit's inclusive line range
/// contains the target symbol's line.
fn is_relevant(hit: &HydratedHit, label: &Label) -> bool {
    hit.file_path == label.file
        && hit.start_line <= label.target_line
        && label.target_line <= hit.end_line
}

/// Aggregate per-query ranks into `recall@k` for each cutoff plus MRR.
fn aggregate(ranks: &[Option<usize>]) -> ModeMetrics {
    let n = ranks.len().max(1) as f64;
    let mut recall = [0.0; CUTOFFS.len()];
    for (ci, &cutoff) in CUTOFFS.iter().enumerate() {
        let hits = ranks
            .iter()
            .filter(|r| matches!(r, Some(rank) if *rank <= cutoff))
            .count();
        recall[ci] = hits as f64 / n;
    }
    let mrr = ranks
        .iter()
        .map(|r| match r {
            Some(rank) => 1.0 / *rank as f64,
            None => 0.0,
        })
        .sum::<f64>()
        / n;
    ModeMetrics { recall, mrr }
}

/// The better (smaller) of two ranks; `None` (a miss) is worst.
fn min_rank(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// True when `candidate` ranks strictly WORSE than `reference` (a miss is rank
/// infinity).
fn worse_than(candidate: Option<usize>, reference: Option<usize>) -> bool {
    match (candidate, reference) {
        (Some(c), Some(r)) => c > r,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (None, None) => false,
    }
}

/// Print the human-readable results table for one slice to stdout.
fn print_report(slice_name: &str, m: &Measurement) {
    println!("apohara-codesearch — external real-OSS benchmark — slice: {slice_name}");
    println!(
        "queries: {} total, {} known-miss ({:.0}%)",
        m.total_queries,
        m.known_miss_queries,
        100.0 * m.known_miss_queries as f64 / m.total_queries as f64
    );
    println!();
    println!("| Mode         | recall@5 | recall@10 |   MRR  |");
    println!("|--------------|----------|-----------|--------|");
    print_row("BM25-only", &m.bm25);
    print_row("vector-only", &m.vector);
    print_row("hybrid (RRF)", &m.hybrid);
    println!();
    println!(
        "queries where hybrid < best single mode: {}",
        m.hybrid_worse_than_best_single
    );
    println!("(recall/MRR are byte-stable across runs — verified by a double measurement pass)");
}

fn print_row(label: &str, mode: &ModeMetrics) {
    println!(
        "| {:<12} |  {:.3}   |   {:.3}   | {:.4} |",
        label, mode.recall[0], mode.recall[1], mode.mrr
    );
}

/// A process-monotonic suffix so the two determinism passes use distinct temp
/// dirs. Not security-sensitive — just uniqueness within one run.
fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frozen labeling-rule invariants, per slice: at least 30% known-miss,
    /// K covers the largest cutoff, and every label's target line is positive.
    #[test]
    fn label_set_invariants() {
        assert!(*CUTOFFS.iter().max().unwrap() <= K, "K must cover all cutoffs");
        for slice in SLICES {
            let total = slice.labels.len();
            let known_miss = slice.labels.iter().filter(|l| l.known_miss).count();
            assert!(
                known_miss as f64 / total as f64 >= 0.30,
                "slice '{}': known-miss share {known_miss}/{total} must be >= 30%",
                slice.name
            );
            for l in slice.labels {
                assert!(
                    l.target_line >= 1,
                    "slice '{}': label '{}' has a non-positive line",
                    slice.name,
                    l.query
                );
                assert!(!l.file.is_empty() && !l.symbol.is_empty());
            }
        }
    }

    /// ripgrep slice is ~20-25 queries; hugo slice is ≥8 (US-2 acceptance).
    #[test]
    fn slice_sizes_match_plan() {
        let ripgrep = SLICES.iter().find(|s| s.subdir == "ripgrep").unwrap();
        let hugo = SLICES.iter().find(|s| s.subdir == "hugo").unwrap();
        assert!(
            (20..=25).contains(&ripgrep.labels.len()),
            "ripgrep slice has {} labels, expected 20-25",
            ripgrep.labels.len()
        );
        assert!(
            hugo.labels.len() >= 8,
            "hugo slice has {} labels, expected >= 8",
            hugo.labels.len()
        );
    }

    /// The relevance rule is line-region based, not chunk-id based.
    #[test]
    fn relevance_rule_is_line_region_based() {
        let label = Label {
            query: "q",
            file: "src/foo.rs",
            symbol: "bar",
            target_line: 20,
            known_miss: false,
            note: "",
        };
        let mk = |file: &str, start: i64, end: i64| HydratedHit {
            chunk_id: format!("{file}:{start}-{end}"),
            file_path: file.to_string(),
            start_line: start,
            end_line: end,
            kind: "function".to_string(),
            signature: None,
            snippet: String::new(),
            imports: vec![],
            exports: vec![],
        };
        assert!(is_relevant(&mk("src/foo.rs", 10, 25), &label));
        assert!(is_relevant(&mk("src/foo.rs", 20, 20), &label));
        assert!(!is_relevant(&mk("src/foo.rs", 1, 19), &label));
        assert!(!is_relevant(&mk("src/foo.rs", 21, 40), &label));
        assert!(!is_relevant(&mk("src/other.rs", 10, 25), &label));
    }

    /// `worse_than` / `min_rank` treat a miss as rank infinity correctly.
    #[test]
    fn rank_comparisons_treat_miss_as_infinity() {
        assert_eq!(min_rank(Some(3), Some(7)), Some(3));
        assert_eq!(min_rank(Some(3), None), Some(3));
        assert_eq!(min_rank(None, None), None);
        assert!(worse_than(Some(13), Some(7)));
        assert!(worse_than(None, Some(7)));
        assert!(!worse_than(Some(7), None));
        assert!(!worse_than(None, None));
        assert!(!worse_than(Some(3), Some(7)));
    }
}
