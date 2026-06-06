// SPDX-License-Identifier: MIT OR Apache-2.0

//! CodeSearchNet `{NL query → code}` retrieval benchmark (one-off, NOT in CI,
//! corpus NOT vendored).
//!
//! This EXTENDS `BENCHMARK.md` with a recognized public dataset. It mirrors
//! `examples/bench-external.rs` in structure and env-gating — env-pointed corpus,
//! never packaged by dist, twice-run byte-identical determinism, relevance by
//! `(file, target_line)` not chunk-id — and DIVERGES only in (a) its corpus
//! loader (CodeSearchNet's `{query, code, path}` JSONL pairs instead of
//! hand-labeled developer queries) and (b) an added `hybrid+MMR` arm.
//!
//! ## Prior art (REQUIRED context)
//!
//! `BENCHMARK.md` already measures BM25-only / vector-only / hybrid-RRF recall@5
//! / MRR across synthetic + ripgrep + hugo + TypeScript + django corpora, under a
//! frozen honesty contract whose headline finding is that hybrid LOSES to
//! BM25-only with the feature-hash backend. This bench adds CodeSearchNet as a
//! NEW labeled slice and reports the SAME honest picture (losses included), not a
//! curated win.
//!
//! ## Why an env-pointed corpus (and never vendored)
//!
//! CodeSearchNet is downloaded ONCE, deliberately, OUTSIDE any cargo target by
//! the user running the standalone fetch helper under `scripts/` (a plain shell
//! script — no `build.rs`, no `[[example]]`, no test invokes it, so no `cargo`
//! command can trigger a network fetch). The harness reads the per-language JSONL
//! files ONLY from local disk via `APOHARA_CSN_ROOT`. If the env var is unset, it
//! prints how to set it and exits 0, so a bare `cargo run --example`
//! never hard-fails a clean tree.
//!
//! ## Honesty contract (identical to bench-external.rs)
//!
//! Relevance is `(file, target_line)`, NOT a `path:start-end` chunk id: a CSN
//! record is materialized to one source file, and a hit COUNTS as relevant when
//! its repo-relative `file_path` equals that file AND its hydrated
//! `[start_line, end_line]` contains the record's `target_line`. So a label
//! survives any chunk-boundary change. The per-slice known-miss rate (records the
//! hybrid ranks outside top-k, or only one mode finds) is reported and asserted
//! at the frozen >= 30% floor — publishing the tool's own failures guards against
//! a self-flattering corpus.
//!
//! ## The four arms
//!
//! BM25-only, vector-only, hybrid-RRF (the three from bench-external) PLUS
//! hybrid+MMR — the diversity re-rank (`mmr_rerank`, `MMR_LAMBDA`) applied AFTER
//! fusion, which answers a distinct question the existing arms cannot: does
//! diversity re-ranking help or hurt recall@5 / MRR on a real NL-query corpus?
//!
//! ## Determinism
//!
//! Each slice's metrics are measured twice and asserted byte-identical before
//! printing. The determinism comes from the deterministic feature-hash embedder +
//! the total RRF tie-break — there is NO RNG / seed in this path (N2).
//!
//! ## Dataset
//!
//! CodeSearchNet, the python / go / javascript-(used as typescript-source) splits
//! published with the CodeSearchNet challenge. Exact release/split + checksum live
//! in `BENCHMARK.md`; the `scripts/` fetch helper is how you obtain them.
//!
//! Run: `APOHARA_CSN_ROOT=/path/to/csn \
//!       cargo run --release --example bench-codesearchnet`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apohara_indexer::{
    active_embedder, bm25_query, hydrate, index_repo, load_embeddings, migrate, mmr_rerank,
    open_db, rrf_fuse, vector_query_with, HydratedHit, EMBED_DIM, MMR_LAMBDA,
};

/// Env var pointing at the directory holding the per-language CSN JSONL files
/// (the one-time manual fetch, never vendored here). Expected layout:
/// `$APOHARA_CSN_ROOT/{python,go,typescript}.jsonl`.
const CSN_ROOT_ENV: &str = "APOHARA_CSN_ROOT";

/// How deep each mode's ranked list is fetched. `recall@5` is read off the same
/// length-`K` list, so `K` must be >= the largest reported cutoff.
const K: usize = 10;

/// The recall cutoffs reported per mode. recall@5 is the headline; recall@10 is
/// kept for parity with bench-external's table shape.
const CUTOFFS: [usize; 2] = [5, 10];

/// Cap on records loaded per language slice. A CSN split is huge (hundreds of
/// thousands of records); indexing all of them per arm is not the point. A bench
/// caps the slice to a deterministic prefix so runs are tractable AND byte-stable
/// (the prefix is taken in file order — JSONL line order is the dataset's order).
const MAX_RECORDS_PER_SLICE: usize = 200;

/// One CSN language slice: the JSONL filename under `CSN_ROOT_ENV` and the source
/// file extension its `code` bodies are materialized with (so `detect_language`
/// parses them as that language).
struct Slice {
    /// Report heading.
    name: &'static str,
    /// JSONL filename under the CSN root.
    jsonl: &'static str,
    /// Extension (no dot) the materialized code files get, e.g. `"py"`.
    ext: &'static str,
}

/// The three first-class slices (N1: TypeScript is a supported language, so it
/// gets its OWN slice; python + go complete the supported-language overlap). Rust
/// has no CSN split — documented as not-covered-by-CSN, covered instead by the
/// existing ripgrep slice in bench-external.
const SLICES: &[Slice] = &[
    Slice {
        name: "python",
        jsonl: "python.jsonl",
        ext: "py",
    },
    Slice {
        name: "go",
        jsonl: "go.jsonl",
        ext: "go",
    },
    Slice {
        name: "typescript",
        jsonl: "typescript.jsonl",
        ext: "ts",
    },
];

/// One labeled query loaded from a CSN record. Relevance is `(file, target_line)`
/// exactly as in bench-external: a hit is relevant iff `hit.file_path == file`
/// and `hit.start_line <= target_line <= hit.end_line`.
struct Label {
    /// The natural-language query (the CSN record's docstring summary).
    query: String,
    /// The repo-relative path of the materialized source file for this record.
    file: String,
    /// Line known to sit INSIDE the materialized code body, so any reasonable
    /// chunk boundary contains it.
    target_line: i64,
}

/// A single CSN JSONL record, decoded with only the fields this bench needs. The
/// CodeSearchNet schema is `{repo, path, func_name, original_string, language,
/// code, code_tokens, docstring, docstring_tokens, url, partition}`; we read
/// `docstring` (the NL query) and `code` (the body to materialize). `path` is
/// read for documentation/traceability only — the on-disk filename this bench
/// generates is deterministic, so relevance does not depend on the dataset path.
#[derive(serde::Deserialize)]
struct CsnRecord {
    docstring: String,
    code: String,
}

fn main() -> Result<()> {
    let Some(root) = std::env::var_os(CSN_ROOT_ENV) else {
        // Unset env is the clean-tree default: print guidance and exit 0 so a
        // bare `cargo run --example bench-codesearchnet` never fails CI-like
        // checks (AC1, mirrors bench-external).
        println!(
            "{CSN_ROOT_ENV} is unset. Point it at the directory holding the per-language\n\
             CodeSearchNet JSONL files (the one-time manual fetch via the scripts/ helper),\n\
             with this layout:\n\n  \
             $CSN_ROOT/python.jsonl\n  \
             $CSN_ROOT/go.jsonl\n  \
             $CSN_ROOT/typescript.jsonl\n\n  \
             {CSN_ROOT_ENV}=/home/you/csn \\\n    \
             cargo run --release --example bench-codesearchnet\n\n\
             The corpus is read ONLY from local disk and is never vendored into this repo.\n\
             To obtain it, run the standalone fetch helper under scripts/ (no cargo target invokes it)."
        );
        return Ok(());
    };
    let root = PathBuf::from(root);

    let mut measured_any = false;
    for slice in SLICES {
        let jsonl = root.join(slice.jsonl);
        if !jsonl.is_file() {
            // A missing slice file is a SKIP, not a failure: the splits are
            // fetched independently, so a partial root must still measure what is
            // present (same spirit as the unset-env branch).
            eprintln!(
                "skipping slice '{}': JSONL not found at {} (fetch it there to measure it)",
                slice.name,
                jsonl.display()
            );
            continue;
        }
        measured_any = true;

        // Determinism gate: measure twice, require byte-identical metrics before
        // printing — same contract as bench-external.rs.
        let first = measure(&jsonl, slice.ext)
            .with_context(|| format!("first measurement pass for slice '{}'", slice.name))?;
        let second = measure(&jsonl, slice.ext)
            .with_context(|| format!("second measurement pass for slice '{}'", slice.name))?;
        assert_eq!(
            first, second,
            "non-deterministic metrics for slice '{}' — recall/MRR must be byte-stable",
            slice.name
        );

        print_report(slice.name, &first);
        println!();
    }

    if !measured_any {
        eprintln!(
            "no slice JSONL found under {} — run the scripts/ fetch helper to populate it first",
            root.display()
        );
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

/// The full measurement for one slice: metrics for each of the four arms plus the
/// count of queries where the hybrid's best relevant rank is strictly worse than
/// the best single mode's, and the known-miss count (hybrid ranks the target
/// outside the top-k list).
#[derive(Debug, Clone, PartialEq)]
struct Measurement {
    bm25: ModeMetrics,
    vector: ModeMetrics,
    hybrid: ModeMetrics,
    hybrid_mmr: ModeMetrics,
    hybrid_worse_than_best_single: usize,
    total_queries: usize,
    known_miss_queries: usize,
}

/// Load the CSN JSONL slice, materialize each record's code to a temp source
/// tree, index it once, then run all four arms for every label and aggregate.
/// Mirrors bench-external.rs::measure; the loader + materialization + MMR arm are
/// the only divergences.
fn measure(jsonl: &Path, ext: &str) -> Result<Measurement> {
    // Index into a temp tree OUTSIDE any tracked dir so nothing is polluted and
    // the walker never sees the database file.
    let tmp = std::env::temp_dir().join(format!(
        "apohara-bench-csn-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let corpus = tmp.join("corpus");
    std::fs::create_dir_all(&corpus).context("create temp corpus dir")?;
    let db_path = tmp.join("bench-index.db");

    let result = (|| -> Result<Measurement> {
        // Materialize records into the corpus tree, building the label table. The
        // filename is deterministic (`rec_{i}.{ext}`) so `(file, target_line)`
        // relevance does not depend on the dataset's own paths.
        let labels = materialize_corpus(jsonl, &corpus, ext)?;

        let conn = open_db(&db_path).context("open temp index db")?;
        migrate(&conn).context("migrate temp index db")?;
        index_repo(&conn, &corpus).context("index materialized CSN corpus")?;

        // Query with the SAME active embedder the index was built with (default:
        // feature-hash; candle BERT when configured via APOHARA_EMBED_MODEL).
        let embedder = active_embedder(EMBED_DIM);

        let mut bm_ranks = Vec::with_capacity(labels.len());
        let mut ve_ranks = Vec::with_capacity(labels.len());
        let mut hy_ranks = Vec::with_capacity(labels.len());
        let mut mmr_ranks = Vec::with_capacity(labels.len());

        // Smallest 1-based rank at which a relevant hit appears in `ids`, or
        // `None`. Relevance is the frozen line-region rule.
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

        for label in &labels {
            let bm = bm25_query(&conn, &label.query, K).context("bm25_query")?;
            let ve = vector_query_with(&conn, &label.query, K, embedder.as_ref())
                .context("vector_query_with")?;
            let hy = rrf_fuse(&bm, &ve, apohara_indexer::RRF_K);

            // hybrid+MMR arm: the diversity re-rank applied AFTER fusion, over the
            // persisted feature-hash vectors — exactly how server.rs applies it.
            let hy_ids_for_mmr: Vec<String> = hy.iter().map(|(id, _)| id.clone()).collect();
            let embeddings = load_embeddings(&conn, &hy_ids_for_mmr).context("load_embeddings")?;
            let mmr = mmr_rerank(&hy, &embeddings, MMR_LAMBDA);

            let bm_ids: Vec<String> = bm.into_iter().map(|(id, _)| id).collect();
            let ve_ids: Vec<String> = ve.into_iter().map(|h| h.chunk_id).collect();
            let hy_ids: Vec<String> = hy.into_iter().map(|(id, _)| id).collect();
            let mmr_ids: Vec<String> = mmr.into_iter().map(|(id, _)| id).collect();

            bm_ranks.push(relevant_rank(&bm_ids, label)?);
            ve_ranks.push(relevant_rank(&ve_ids, label)?);
            hy_ranks.push(relevant_rank(&hy_ids, label)?);
            mmr_ranks.push(relevant_rank(&mmr_ids, label)?);
        }

        let hybrid_worse = (0..labels.len())
            .filter(|&i| {
                let best_single = min_rank(bm_ranks[i], ve_ranks[i]);
                worse_than(hy_ranks[i], best_single)
            })
            .count();

        // Known-miss: the hybrid ranks the relevant target OUTSIDE the top-k list
        // (or never), measured at run time rather than hand-committed (the CSN
        // records are loaded, not authored). The contract is the SAME >=30% floor.
        let known_miss = hy_ranks.iter().filter(|r| r.is_none()).count();

        Ok(Measurement {
            bm25: aggregate(&bm_ranks),
            vector: aggregate(&ve_ranks),
            hybrid: aggregate(&hy_ranks),
            hybrid_mmr: aggregate(&mmr_ranks),
            hybrid_worse_than_best_single: hybrid_worse,
            total_queries: labels.len(),
            known_miss_queries: known_miss,
        })
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Read the JSONL slice, materialize each record's `code` into `corpus` as one
/// source file (`rec_{i}.{ext}`), and return the label table. Records with an
/// empty docstring or empty code are skipped (no usable query / nothing to
/// index). Loading stops at [`MAX_RECORDS_PER_SLICE`] usable records so the run
/// is tractable AND deterministic (a fixed file-order prefix).
fn materialize_corpus(jsonl: &Path, corpus: &Path, ext: &str) -> Result<Vec<Label>> {
    let file = std::fs::File::open(jsonl).context("open CSN JSONL")?;
    let reader = BufReader::new(file);

    let mut labels = Vec::new();
    // Fail-soft on a malformed line, consistent with the rest of this loader
    // (missing slice / empty record are skipped, not fatal). A truncated JSONL
    // line is a realistic outcome of a partial HuggingFace shard download, and
    // aborting the whole slice over one bad row would be the only fail-loud path
    // in an otherwise fail-soft file. Count + report to stderr instead.
    let mut malformed = 0usize;
    for line in reader.lines() {
        let line = line.context("read CSN JSONL line")?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: CsnRecord = match serde_json::from_str(line) {
            Ok(rec) => rec,
            Err(_) => {
                malformed += 1;
                continue;
            }
        };
        let query = first_line(&rec.docstring);
        if query.is_empty() || rec.code.trim().is_empty() {
            continue;
        }

        let idx = labels.len();
        let rel_path = format!("rec_{idx}.{ext}");
        let abs_path = corpus.join(&rel_path);
        // A leading blank line guarantees the body (and its first real line) sits
        // at line >= 2, INSIDE any reasonable chunk window — the same "target a
        // line inside the body" discipline bench-external uses for hand labels.
        let body = format!("\n{}\n", rec.code.trim_end());
        std::fs::write(&abs_path, &body).context("write materialized CSN file")?;

        labels.push(Label {
            query,
            file: rel_path,
            // Line 2 is the first line of the actual code (after the leading
            // blank), always inside the function body's chunk.
            target_line: 2,
        });

        if labels.len() >= MAX_RECORDS_PER_SLICE {
            break;
        }
    }
    if malformed > 0 {
        eprintln!(
            "  warning: skipped {malformed} malformed JSONL line(s) in {}",
            jsonl.display()
        );
    }
    Ok(labels)
}

/// First non-empty line of a docstring, trimmed — the CSN challenge's convention
/// for the natural-language query (the summary line, not the full docstring).
fn first_line(docstring: &str) -> String {
    docstring
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// The frozen relevance predicate: same file AND the hit's inclusive line range
/// contains the target line.
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
    println!("apohara-codesearch — CodeSearchNet benchmark — slice: {slice_name}");
    println!(
        "queries: {} total, {} known-miss ({:.0}%)",
        m.total_queries,
        m.known_miss_queries,
        100.0 * m.known_miss_queries as f64 / m.total_queries.max(1) as f64
    );
    println!();
    println!("| Mode         | recall@5 | recall@10 |   MRR  |");
    println!("|--------------|----------|-----------|--------|");
    print_row("BM25-only", &m.bm25);
    print_row("vector-only", &m.vector);
    print_row("hybrid (RRF)", &m.hybrid);
    print_row("hybrid+MMR", &m.hybrid_mmr);
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

    /// AC3: recall@5 and MRR on a fixture of KNOWN ranks, asserting EXACT values.
    ///
    /// Ranks: [1, 3, 6, None]. With CUTOFFS = [5, 10]:
    ///   recall@5  = 2/4 = 0.5  (ranks 1 and 3 are <= 5; 6 and miss are not)
    ///   recall@10 = 3/4 = 0.75 (1, 3, 6 are <= 10)
    ///   MRR = (1/1 + 1/3 + 1/6 + 0) / 4 = (1 + 0.333.. + 0.166..) / 4 = 0.375
    #[test]
    fn metrics_on_known_ranks_have_exact_values() {
        let ranks = [Some(1usize), Some(3), Some(6), None];
        let m = aggregate(&ranks);
        assert_eq!(m.recall[0], 0.5, "recall@5 for ranks [1,3,6,None]");
        assert_eq!(m.recall[1], 0.75, "recall@10 for ranks [1,3,6,None]");
        // (1 + 1/3 + 1/6) / 4 = 1.5 / 4 = 0.375 exactly.
        assert_eq!(m.mrr, 0.375, "MRR for ranks [1,3,6,None]");
    }

    /// A perfect-rank fixture: every query at rank 1 → recall@5 = recall@10 = MRR
    /// = 1.0 exactly.
    #[test]
    fn metrics_all_rank_one_are_perfect() {
        let ranks = [Some(1usize), Some(1), Some(1)];
        let m = aggregate(&ranks);
        assert_eq!(m.recall[0], 1.0);
        assert_eq!(m.recall[1], 1.0);
        assert_eq!(m.mrr, 1.0);
    }

    /// An all-miss fixture: recall and MRR are all 0.0 exactly.
    #[test]
    fn metrics_all_miss_are_zero() {
        let ranks = [None, None];
        let m = aggregate(&ranks);
        assert_eq!(m.recall[0], 0.0);
        assert_eq!(m.recall[1], 0.0);
        assert_eq!(m.mrr, 0.0);
    }

    /// A rank just past the recall@5 cutoff but within recall@10: recall@5 = 0,
    /// recall@10 = 1, MRR = 1/6. Pins the cutoff boundary exactly.
    #[test]
    fn metrics_cutoff_boundary_is_exact() {
        let ranks = [Some(6usize)];
        let m = aggregate(&ranks);
        assert_eq!(m.recall[0], 0.0, "rank 6 is past the recall@5 cutoff");
        assert_eq!(m.recall[1], 1.0, "rank 6 is within the recall@10 cutoff");
        assert_eq!(m.mrr, 1.0 / 6.0);
    }

    /// `worse_than` / `min_rank` treat a miss as rank infinity correctly (the
    /// hybrid-worse-than-best-single count depends on this).
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

    /// The relevance rule is line-region based, not chunk-id based.
    #[test]
    fn relevance_rule_is_line_region_based() {
        let label = Label {
            query: "q".to_string(),
            file: "rec_0.py".to_string(),
            target_line: 2,
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
        assert!(is_relevant(&mk("rec_0.py", 1, 10), &label));
        assert!(is_relevant(&mk("rec_0.py", 2, 2), &label));
        assert!(!is_relevant(&mk("rec_0.py", 3, 9), &label));
        assert!(!is_relevant(&mk("rec_1.py", 1, 10), &label));
    }

    /// `first_line` returns the first non-empty trimmed line — the CSN NL query.
    #[test]
    fn first_line_picks_the_summary() {
        assert_eq!(
            first_line("Sums two numbers.\n\nMore detail."),
            "Sums two numbers."
        );
        assert_eq!(first_line("\n   \n  trimmed me  \nrest"), "trimmed me");
        assert_eq!(first_line(""), "");
        assert_eq!(first_line("   \n  "), "");
    }
}
