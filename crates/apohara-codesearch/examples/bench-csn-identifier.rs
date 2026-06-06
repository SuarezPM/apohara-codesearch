// SPDX-License-Identifier: MIT OR Apache-2.0

//! CodeSearchNet `{identifier query → code}` retrieval benchmark — the AC4
//! recovery-gate slice (one-off, NOT in CI, corpus NOT vendored).
//!
//! ## Why a SECOND CSN bench (vs `bench-codesearchnet.rs`)
//!
//! The sibling `bench-codesearchnet` uses the CSN `docstring` (a
//! natural-language summary) as the query. This bench instead uses the CSN
//! `func_name` — the function's IDENTIFIER (e.g. `get_vid_from_url`,
//! `YouTube.parse_url`) — as the query, with the SAME `code` body as the
//! retrieval target. Identifier-shaped queries are the case the adaptive-fusion
//! heuristic (`resolve_weights` → `classify_query_weights`) is built for: a
//! single bare token carrying `_`/digits/a case boundary up-weights BM25 (the
//! lexical exact-symbol lookup), while a multi-word NL phrase up-weights vector.
//!
//! ## AC4 recovery gate (the question this bench answers)
//!
//! With a REAL learned embedder (EmbeddingGemma, `--features gguf-embed` +
//! `APOHARA_EMBED_MODEL`), does the ADAPTIVE arm — which biases BM25-heavy on
//! identifier-shaped queries — recall@5 AT LEAST as well as BOTH:
//!   (a) BM25-only, AND
//!   (b) plain (equal-weight) hybrid?
//! i.e. `adaptive_recall@5 >= bm25_recall@5  AND  adaptive_recall@5 >= hybrid_recall@5`.
//! The verdict (GO / no-go) is printed per slice from the MEASURED numbers — an
//! honest result, win or loss, exactly as `BENCHMARK.md` commits to.
//!
//! ## Prompt fidelity (a deliberate choice, documented)
//!
//! The query is embedded through the SAME `embed_query` path server.rs takes in
//! production — for EmbeddingGemma that prepends the asymmetric QUERY prompt to
//! the identifier. AC4 asks whether the SYSTEM AS DEPLOYED recovers on
//! identifier queries via adaptive fusion, so the bench mirrors the deployed
//! query path rather than stripping the prompt; stripping it would measure a
//! configuration the server never runs.
//!
//! ## Everything else is identical to bench-codesearchnet.rs
//!
//! Same `(file, target_line)` relevance, same five arms
//! (BM25/vector/hybrid/hybrid+MMR/adaptive), same twice-run byte-identical
//! determinism gate, same per-slice cap, same fail-soft loader. The corpus is
//! read ONLY from local disk via `APOHARA_CSN_IDENT_ROOT`; if unset, it prints
//! guidance and exits 0 (never a network fetch in any cargo command).
//!
//! Run: `APOHARA_EMBED_MODEL=/path/to/eg-st \
//!       APOHARA_CSN_IDENT_ROOT=/path/to/csn-ident \
//!       cargo run --release --features gguf-embed --example bench-csn-identifier`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apohara_indexer::{
    active_embedder, bm25_query, hydrate, index_repo, load_embeddings, migrate, mmr_rerank,
    open_db_with, resolve_weights, rrf_fuse, rrf_fuse_weighted, vector_query_with, HydratedHit,
    EMBED_DIM, MMR_LAMBDA,
};

/// Env var pointing at the directory holding the per-language IDENTIFIER JSONL
/// files (`{docstring: func_name, code: func_code_string}` per line, where the
/// `docstring` slot carries the identifier query). Expected layout:
/// `$APOHARA_CSN_IDENT_ROOT/{python,go,typescript}.jsonl`.
const CSN_IDENT_ROOT_ENV: &str = "APOHARA_CSN_IDENT_ROOT";

/// How deep each mode's ranked list is fetched. `recall@5` is read off the same
/// length-`K` list, so `K` must be >= the largest reported cutoff.
const K: usize = 10;

/// The recall cutoffs reported per mode. recall@5 is the AC4 headline; recall@10
/// is kept for parity with the sibling bench's table shape.
const CUTOFFS: [usize; 2] = [5, 10];

/// Cap on records loaded per language slice (deterministic file-order prefix).
/// Identifier queries through a real 300M embedder on CPU are heavy, so this is
/// intentionally smaller than the NL bench's 200; bump it for a deeper run.
const MAX_RECORDS_PER_SLICE: usize = 100;

/// One CSN language slice: the JSONL filename under `CSN_IDENT_ROOT_ENV` and the
/// source file extension its `code` bodies are materialized with.
struct Slice {
    name: &'static str,
    jsonl: &'static str,
    ext: &'static str,
}

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
/// exactly as in the sibling bench.
struct Label {
    /// The IDENTIFIER query (the CSN record's `func_name`).
    query: String,
    file: String,
    target_line: i64,
}

/// A single JSONL record. The identifier slices store the func_name in the
/// `docstring` slot (so the on-disk schema matches the sibling bench's
/// `{docstring, code}` and one loader serves both); `docstring` therefore holds
/// the identifier query here, NOT a natural-language summary.
#[derive(serde::Deserialize)]
struct CsnRecord {
    docstring: String,
    code: String,
}

fn main() -> Result<()> {
    let Some(root) = std::env::var_os(CSN_IDENT_ROOT_ENV) else {
        println!(
            "{CSN_IDENT_ROOT_ENV} is unset. Point it at the directory holding the per-language\n\
             identifier JSONL files ({{\"docstring\": func_name, \"code\": func_code_string}} per\n\
             line), with this layout:\n\n  \
             $CSN_IDENT_ROOT/python.jsonl\n  \
             $CSN_IDENT_ROOT/go.jsonl\n  \
             $CSN_IDENT_ROOT/typescript.jsonl\n\n  \
             APOHARA_EMBED_MODEL=/path/to/eg-st \\\n    \
             {CSN_IDENT_ROOT_ENV}=/home/you/csn-ident \\\n    \
             cargo run --release --features gguf-embed --example bench-csn-identifier\n\n\
             The corpus is read ONLY from local disk and is never vendored into this repo."
        );
        return Ok(());
    };
    let root = PathBuf::from(root);

    let mut measured_any = false;
    for slice in SLICES {
        let jsonl = root.join(slice.jsonl);
        if !jsonl.is_file() {
            eprintln!(
                "skipping slice '{}': JSONL not found at {} (extract it there to measure it)",
                slice.name,
                jsonl.display()
            );
            continue;
        }
        measured_any = true;

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
            "no slice JSONL found under {} — extract the identifier slices there first",
            root.display()
        );
    }

    Ok(())
}

/// Per-mode aggregate metrics. Equality drives the determinism gate.
#[derive(Debug, Clone, PartialEq)]
struct ModeMetrics {
    recall: [f64; CUTOFFS.len()],
    mrr: f64,
}

/// The full measurement for one slice: metrics for each of the five arms plus
/// the hybrid-worse-than-best-single count and the known-miss count.
#[derive(Debug, Clone, PartialEq)]
struct Measurement {
    bm25: ModeMetrics,
    vector: ModeMetrics,
    hybrid: ModeMetrics,
    hybrid_mmr: ModeMetrics,
    /// The AC4 recovery arm: per-query adaptive weights from `resolve_weights`
    /// (heuristic on). Identifier-shaped single tokens up-weight BM25.
    adaptive: ModeMetrics,
    hybrid_worse_than_best_single: usize,
    total_queries: usize,
    known_miss_queries: usize,
}

/// Load the JSONL slice, materialize each record's code, index once, then run
/// all five arms for every identifier label and aggregate. Mirrors
/// bench-codesearchnet.rs::measure exactly; only the query source (func_name)
/// differs, and that difference lives entirely in the JSONL contents.
fn measure(jsonl: &Path, ext: &str) -> Result<Measurement> {
    let tmp = std::env::temp_dir().join(format!(
        "apohara-bench-csn-ident-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let corpus = tmp.join("corpus");
    std::fs::create_dir_all(&corpus).context("create temp corpus dir")?;
    let db_path = tmp.join("bench-index.db");

    let result = (|| -> Result<Measurement> {
        let labels = materialize_corpus(jsonl, &corpus, ext)?;

        // Same active embedder the index was built with (feature-hash by default;
        // EmbeddingGemma when APOHARA_EMBED_MODEL points at the checkpoint).
        let embedder = active_embedder(EMBED_DIM);
        let conn = open_db_with(&db_path, embedder.dim()).context("open temp index db")?;
        migrate(&conn).context("migrate temp index db")?;
        index_repo(&conn, &corpus).context("index materialized CSN corpus")?;

        let mut bm_ranks = Vec::with_capacity(labels.len());
        let mut ve_ranks = Vec::with_capacity(labels.len());
        let mut hy_ranks = Vec::with_capacity(labels.len());
        let mut mmr_ranks = Vec::with_capacity(labels.len());
        let mut ad_ranks = Vec::with_capacity(labels.len());

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

            let hy_ids_for_mmr: Vec<String> = hy.iter().map(|(id, _)| id.clone()).collect();
            let embeddings = load_embeddings(&conn, &hy_ids_for_mmr).context("load_embeddings")?;
            let mmr = mmr_rerank(&hy, &embeddings, MMR_LAMBDA);

            // adaptive arm: per-query weights from the query SHAPE (identifier →
            // BM25-heavy), then weighted fusion — the server.rs adaptive path.
            let (ad_bm25_w, ad_vec_w) = resolve_weights(None, None, true, &label.query);
            let ad = rrf_fuse_weighted(&bm, &ve, apohara_indexer::RRF_K, ad_bm25_w, ad_vec_w);

            let bm_ids: Vec<String> = bm.into_iter().map(|(id, _)| id).collect();
            let ve_ids: Vec<String> = ve.into_iter().map(|h| h.chunk_id).collect();
            let hy_ids: Vec<String> = hy.into_iter().map(|(id, _)| id).collect();
            let mmr_ids: Vec<String> = mmr.into_iter().map(|(id, _)| id).collect();
            let ad_ids: Vec<String> = ad.into_iter().map(|(id, _)| id).collect();

            bm_ranks.push(relevant_rank(&bm_ids, label)?);
            ve_ranks.push(relevant_rank(&ve_ids, label)?);
            hy_ranks.push(relevant_rank(&hy_ids, label)?);
            mmr_ranks.push(relevant_rank(&mmr_ids, label)?);
            ad_ranks.push(relevant_rank(&ad_ids, label)?);
        }

        let hybrid_worse = (0..labels.len())
            .filter(|&i| {
                let best_single = min_rank(bm_ranks[i], ve_ranks[i]);
                worse_than(hy_ranks[i], best_single)
            })
            .count();

        let known_miss = hy_ranks.iter().filter(|r| r.is_none()).count();

        Ok(Measurement {
            bm25: aggregate(&bm_ranks),
            vector: aggregate(&ve_ranks),
            hybrid: aggregate(&hy_ranks),
            hybrid_mmr: aggregate(&mmr_ranks),
            adaptive: aggregate(&ad_ranks),
            hybrid_worse_than_best_single: hybrid_worse,
            total_queries: labels.len(),
            known_miss_queries: known_miss,
        })
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Read the JSONL slice, materialize each record's `code` into `corpus` as one
/// source file, and return the label table. The `docstring` slot carries the
/// identifier query. Records with an empty query or empty code are skipped.
fn materialize_corpus(jsonl: &Path, corpus: &Path, ext: &str) -> Result<Vec<Label>> {
    let file = std::fs::File::open(jsonl).context("open CSN identifier JSONL")?;
    let reader = BufReader::new(file);

    let mut labels = Vec::new();
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
        // The identifier query: the func_name stored in the docstring slot,
        // first non-empty line, trimmed.
        let query = first_line(&rec.docstring);
        if query.is_empty() || rec.code.trim().is_empty() {
            continue;
        }

        let idx = labels.len();
        let rel_path = format!("rec_{idx}.{ext}");
        let abs_path = corpus.join(&rel_path);
        let body = format!("\n{}\n", rec.code.trim_end());
        std::fs::write(&abs_path, &body).context("write materialized CSN file")?;

        labels.push(Label {
            query,
            file: rel_path,
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

/// First non-empty line of the query field, trimmed.
fn first_line(s: &str) -> String {
    s.lines()
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

fn min_rank(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn worse_than(candidate: Option<usize>, reference: Option<usize>) -> bool {
    match (candidate, reference) {
        (Some(c), Some(r)) => c > r,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (None, None) => false,
    }
}

/// Print the human-readable results table for one slice, plus the AC4 verdict.
fn print_report(slice_name: &str, m: &Measurement) {
    println!("apohara-codesearch — CSN IDENTIFIER benchmark (AC4) — slice: {slice_name}");
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
    print_row("adaptive", &m.adaptive);
    println!();
    println!(
        "queries where hybrid < best single mode: {}",
        m.hybrid_worse_than_best_single
    );

    // AC4 recovery gate: adaptive recall@5 must be >= BM25-only AND >= plain
    // hybrid, measured on identifier-shaped queries.
    let ad = m.adaptive.recall[0];
    let bm = m.bm25.recall[0];
    let hy = m.hybrid.recall[0];
    let vs_bm25 = ad >= bm;
    let vs_hybrid = ad >= hy;
    let verdict = if vs_bm25 && vs_hybrid { "GO" } else { "NO-GO" };
    println!(
        "AC4 [{verdict}]: adaptive recall@5 {ad:.3} >= BM25 {bm:.3} ({}) AND >= hybrid {hy:.3} ({})",
        if vs_bm25 { "ok" } else { "FAIL" },
        if vs_hybrid { "ok" } else { "FAIL" },
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

    /// recall@5 / recall@10 / MRR on a fixture of KNOWN ranks, exact values.
    /// Ranks [1, 3, 6, None]: recall@5 = 0.5, recall@10 = 0.75, MRR = 0.375.
    #[test]
    fn metrics_on_known_ranks_have_exact_values() {
        let ranks = [Some(1usize), Some(3), Some(6), None];
        let m = aggregate(&ranks);
        assert_eq!(m.recall[0], 0.5);
        assert_eq!(m.recall[1], 0.75);
        assert_eq!(m.mrr, 0.375);
    }

    /// `first_line` returns the first non-empty trimmed line — the identifier.
    #[test]
    fn first_line_picks_the_identifier() {
        assert_eq!(first_line("get_vid_from_url"), "get_vid_from_url");
        assert_eq!(
            first_line("\n  YouTube.parse_url  \nrest"),
            "YouTube.parse_url"
        );
        assert_eq!(first_line(""), "");
    }

    /// The relevance rule is line-region based, not chunk-id based.
    #[test]
    fn relevance_rule_is_line_region_based() {
        let label = Label {
            query: "get_vid".to_string(),
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
        assert!(!is_relevant(&mk("rec_0.py", 3, 9), &label));
        assert!(!is_relevant(&mk("rec_1.py", 1, 10), &label));
    }
}
