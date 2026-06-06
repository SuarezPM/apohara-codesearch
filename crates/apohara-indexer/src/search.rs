// SPDX-License-Identifier: MIT OR Apache-2.0

//! Query-time retrieval: lexical (BM25 over FTS5), vector (sqlite-vec KNN),
//! Reciprocal Rank Fusion of the two, and hydration of a fused hit into a
//! displayable record.
//!
//! The lexical and vector paths each rank chunks independently; `rrf_fuse`
//! merges them by rank (not by raw score, which lives on incomparable scales),
//! and `hydrate` joins a winning `chunk_id` back to its source location,
//! signature, snippet, and the file's structural imports/exports.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::storage::{self, KnnHit};
use crate::tokens::code_tokens;

/// Default RRF constant. 60 is the canonical value from Cormack et al. 2009;
/// larger `k_rrf` flattens the contribution of top ranks (less aggressive
/// reordering), smaller sharpens it.
pub const RRF_K: usize = 60;

/// Default Maximal Marginal Relevance trade-off. `lambda`
/// weights relevance vs. diversity in `lambda * rel - (1-lambda) * max_sim`;
/// 0.7 keeps a strong relevance bias while still demoting near-duplicates.
/// Tunable.
pub const MMR_LAMBDA: f64 = 0.7;

/// Additive fused-score boost applied to a file's chunks when a query token
/// matches one of that file's persisted `file_imports.source` values.
/// Small on purpose: it nudges import-relevant files up without
/// overriding the RRF signal. Tunable.
pub const STRUCTURAL_BOOST: f64 = 0.01;

/// Maximum snippet length in characters. Long bodies are truncated on a char
/// boundary so callers receive a bounded, UTF-8-safe preview.
pub const SNIPPET_MAX: usize = 2000;

/// Lexical retrieval over the contentless `chunks_fts` index. Returns up to `k`
/// `(chunk_id, bm25_score)` pairs, best-first.
///
/// FTS5 `bm25()` returns a NEGATIVE relevance score (more-negative = better
/// match), so `ORDER BY score ASC` yields best-first and the caller can use the
/// score column directly for tie-breaking.
///
/// Degenerate-query contract: an empty `query`, or one whose `code_tokens`
/// expansion is empty (all punctuation / separators), returns `Ok(vec![])`
/// WITHOUT running a MATCH — FTS5 raises on empty/malformed match expressions,
/// so we never hand it one.
pub fn bm25_query(conn: &Connection, query: &str, k: usize) -> Result<Vec<(String, f64)>> {
    let tokens = code_tokens(query);
    if query.is_empty() || tokens.is_empty() {
        return Ok(vec![]);
    }

    // OR-join the bare tokens, each wrapped as an FTS5 string literal
    // ("token"). code_tokens already yields alphanumeric-only tokens, so the
    // quoting is belt-and-suspenders: it guarantees no token char is ever
    // interpreted as an FTS5 operator.
    let match_expr = tokens
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ");

    // bm25() is negative (more-negative = better), so ASC = best-first.
    let mut stmt = conn
        .prepare(
            "SELECT c.id, bm25(chunks_fts) AS score \
             FROM chunks_fts JOIN chunks c ON c.rowid = chunks_fts.rowid \
             WHERE chunks_fts MATCH ?1 \
             ORDER BY score ASC LIMIT ?2",
        )
        .context("prepare bm25 statement")?;
    let rows = stmt
        .query_map(params![match_expr, k as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .context("execute bm25 query")?;

    let mut hits = Vec::new();
    for r in rows {
        hits.push(r.context("read bm25 row")?);
    }
    Ok(hits)
}

/// Vector retrieval. Thin wrapper over [`storage::knn_query`], which embeds the
/// query with the same feature-hashing pipeline and JOINs vec0 results back to
/// chunk ids. Returns `KnnHit`s in ascending distance order (best-first).
pub fn vector_query(conn: &Connection, query: &str, k: usize) -> Result<Vec<KnnHit>> {
    storage::knn_query(conn, query, k)
}

/// Embedder-aware vector retrieval. Thin wrapper over
/// [`storage::knn_query_with`]: embeds `query` with the supplied active
/// [`Embedder`] (which MUST match the embedder the index was built with — the
/// caller enforces this via [`crate::schema::verify_embedder_meta`]).
pub fn vector_query_with(
    conn: &Connection,
    query: &str,
    k: usize,
    embedder: &dyn crate::embedder::Embedder,
) -> Result<Vec<KnnHit>> {
    storage::knn_query_with(conn, query, k, embedder)
}

/// Reciprocal Rank Fusion of a lexical and a vector ranked list.
///
/// `score(id) = Σ_lists 1 / (k_rrf + rank)` with 1-based rank within each list
/// and equal weight per list; a chunk appearing in both lists sums both
/// contributions. Pass [`RRF_K`] for the canonical default.
///
/// Deterministic ordering (P1-2): results are sorted by
/// `(fused_score DESC, bm25_rank ASC, chunk_id ASC)`. `bm25_rank` is the id's
/// 1-based position in the bm25 list (or a large sentinel when absent), and the
/// final lexicographic `chunk_id` tie-break makes the top-k fully stable across
/// runs.
///
/// This is the backward-compatible entry point: it delegates to
/// [`rrf_fuse_weighted`] with equal weights `1.0 / 1.0`, which reproduces the
/// unweighted behavior exactly (the canonical `rrf_proof.rs` regression guard).
pub fn rrf_fuse(bm25: &[(String, f64)], vector: &[KnnHit], k_rrf: usize) -> Vec<(String, f64)> {
    rrf_fuse_weighted(bm25, vector, k_rrf, 1.0, 1.0)
}

/// Weighted Reciprocal Rank Fusion.
///
/// `score(id) = Σ_lists w_list / (k_rrf + rank)` with 1-based rank within each
/// list; a chunk appearing in both lists sums both weighted contributions.
/// `bm25_weight` scales the lexical list's contribution, `vector_weight` the
/// vector list's.
///
/// EQUAL weights (`1.0, 1.0`) with `k_rrf = RRF_K` reproduce the exact
/// unweighted ordering: the per-list `w / (k_rrf + rank)` term collapses to the
/// original `1 / (k_rrf + rank)`, and the deterministic tie-break
/// `(fused_score DESC, bm25_rank ASC, chunk_id ASC)` is unchanged. [`rrf_fuse`]
/// is exactly this call with `1.0 / 1.0`.
pub fn rrf_fuse_weighted(
    bm25: &[(String, f64)],
    vector: &[KnnHit],
    k_rrf: usize,
    bm25_weight: f64,
    vector_weight: f64,
) -> Vec<(String, f64)> {
    // bm25_rank: 1-based rank of each id in the lexical list, for tie-breaking.
    let mut bm25_rank: HashMap<&str, usize> = HashMap::new();
    for (rank, (id, _)) in bm25.iter().enumerate() {
        bm25_rank.entry(id.as_str()).or_insert(rank + 1);
    }

    // Accumulate fused scores. First-seen rank wins if an id repeats in a list.
    let mut fused: HashMap<String, f64> = HashMap::new();
    let mut seen_bm25: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (rank, (id, _)) in bm25.iter().enumerate() {
        if seen_bm25.insert(id.as_str()) {
            *fused.entry(id.clone()).or_insert(0.0) += bm25_weight / (k_rrf + rank + 1) as f64;
        }
    }
    let mut seen_vec: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (rank, hit) in vector.iter().enumerate() {
        if seen_vec.insert(hit.chunk_id.as_str()) {
            *fused.entry(hit.chunk_id.clone()).or_insert(0.0) +=
                vector_weight / (k_rrf + rank + 1) as f64;
        }
    }

    // Sentinel rank for ids absent from the bm25 list: larger than any real rank.
    let sentinel = bm25.len() + vector.len() + 1;
    let mut out: Vec<(String, f64)> = fused.into_iter().collect();
    out.sort_by(|a, b| {
        // Primary: fused score DESC.
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Secondary: bm25_rank ASC.
            .then_with(|| {
                let ra = bm25_rank.get(a.0.as_str()).copied().unwrap_or(sentinel);
                let rb = bm25_rank.get(b.0.as_str()).copied().unwrap_or(sentinel);
                ra.cmp(&rb)
            })
            // Tertiary: chunk_id ASC (lexicographic).
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}

/// Adaptive-fusion weights, biased BM25-heavy for queries that look like a
/// lexical/identifier lookup. Used only when no caller weight is set.
const ADAPTIVE_BM25_HEAVY: (f64, f64) = (2.0, 1.0);

/// Adaptive-fusion weights, biased vector-heavy for multi-word natural-language
/// phrases (the conceptual end of the spectrum).
const ADAPTIVE_VECTOR_HEAVY: (f64, f64) = (1.0, 2.0);

/// Adaptive-fusion neutral weights when the query gives no clear lexical signal.
const ADAPTIVE_NEUTRAL: (f64, f64) = (1.0, 1.0);

/// Pick `(bm25_weight, vector_weight)` from the SHAPE of the query string.
///
/// Heuristic, in priority order:
/// - A single bare token that is an identifier (snake_case, camelCase, or any
///   token carrying `_`/digits/a case boundary) → BM25-heavy: it is almost
///   certainly an exact-symbol lookup the lexical index serves best.
/// - A single plain lowercase word → neutral: a one-word query is ambiguous
///   between a symbol and a concept, so we do not bias either way.
/// - Multiple words → vector-heavy: a multi-word phrase reads as a
///   natural-language description, the conceptual case the vector arm targets.
///
/// KNOWN LIMITATION (lexical-only, no corpus signal). This looks ONLY at the
/// query string; it never consults `symbols.name` or any index statistic. So it
/// cannot disambiguate a token that is BOTH a real identifier AND a concept —
/// e.g. `parseConfig` is camelCase (→ BM25-heavy here) yet also a plausible NL
/// concept, and without knowing whether `parseConfig` actually exists in the
/// corpus the choice is a guess. Worse, with the feature-hash embedder that is
/// the default backend the vector arm is near-noise (see `BENCHMARK.md`), so the
/// vector-heavy branch can only help once a real learned embedder is wired. Treat
/// this as a cheap prior, not a classifier — which is why adaptive fusion is
/// opt-in and OFF by default.
pub fn classify_query_weights(query: &str) -> (f64, f64) {
    let words: Vec<&str> = query.split_whitespace().collect();
    match words.as_slice() {
        // Empty / whitespace-only: nothing to bias on.
        [] => ADAPTIVE_NEUTRAL,
        // Exactly one token: BM25-heavy only when it looks like an identifier.
        [single] => {
            if looks_like_identifier(single) {
                ADAPTIVE_BM25_HEAVY
            } else {
                ADAPTIVE_NEUTRAL
            }
        }
        // Multiple words read as a natural-language phrase → vector-heavy.
        _ => ADAPTIVE_VECTOR_HEAVY,
    }
}

/// A token "looks like an identifier" when it carries a structural signal a
/// plain English word would not: an underscore (snake_case), a digit, or an
/// internal lower→upper case boundary (camelCase / PascalCase). A bare
/// all-lowercase word (e.g. `parse`) is intentionally NOT treated as an
/// identifier — it is indistinguishable from an ordinary search term.
///
/// Intentionally NOT covered (fall through to neutral, by design): a
/// single-component PascalCase token (`User` has no internal lower→upper
/// boundary) and `kebab-case` (the 4 supported languages use snake/camel for
/// identifiers, not kebab) — both rare enough as exact-symbol queries that the
/// neutral default is the safe choice over a false BM25 bias.
fn looks_like_identifier(token: &str) -> bool {
    if token.contains('_') || token.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    // Internal case boundary: a lowercase char immediately followed by uppercase.
    token
        .chars()
        .zip(token.chars().skip(1))
        .any(|(a, b)| a.is_lowercase() && b.is_uppercase())
}

/// Resolve the final `(bm25_weight, vector_weight)` for a fusion call, keeping
/// the caller's `Option<f64>` intentions alive past the adaptive decision.
///
/// Precedence (HIGH→LOW), evaluated per axis so an explicit weight on one axis
/// does not force the other:
/// 1. An explicit `Some(w)` from the caller always wins — even with `adaptive`
///    on, an explicit weight is never overridden by the heuristic.
/// 2. Otherwise, when `adaptive` is true, the unset axes come from
///    [`classify_query_weights`].
/// 3. Otherwise the unset axes default to `1.0` (the byte-identical legacy
///    behavior — see `rrf_fuse_weighted_equal_weights_matches_current`).
///
/// Each resolved axis is sanitized to a finite, non-negative weight: a caller
/// (the MCP boundary) could pass a negative weight — which would silently INVERT
/// the rank contribution — or a non-finite one — which would destabilize the
/// sort. Negative → `0.0` (that arm simply does not contribute), non-finite →
/// `1.0` (neutral). The legacy `1.0` default and any sane explicit weight pass
/// through unchanged, so this does not affect the byte-identical regression.
pub fn resolve_weights(
    bm25_weight: Option<f64>,
    vector_weight: Option<f64>,
    adaptive: bool,
    query: &str,
) -> (f64, f64) {
    // Heuristic is consulted ONLY when adaptive is on; otherwise the unset-axis
    // fallback is the legacy 1.0/1.0 default.
    let (adaptive_bm25, adaptive_vec) = if adaptive {
        classify_query_weights(query)
    } else {
        ADAPTIVE_NEUTRAL
    };
    (
        sanitize_weight(bm25_weight.unwrap_or(adaptive_bm25)),
        sanitize_weight(vector_weight.unwrap_or(adaptive_vec)),
    )
}

/// Coerce a fusion weight to a usable value: non-finite (NaN/±inf) → `1.0`
/// (neutral, keeps the sort stable); negative → `0.0` (drops that arm rather
/// than inverting its contribution). Finite non-negative weights are returned
/// unchanged.
fn sanitize_weight(w: f64) -> f64 {
    if !w.is_finite() {
        1.0
    } else if w < 0.0 {
        0.0
    } else {
        w
    }
}

/// Stage-A overlap threshold (1c): two same-file ranges that overlap by MORE
/// than this fraction of the SHORTER range are treated as near-duplicates.
/// Strict `>` against this constant — an exact-50% window-boundary overlap is
/// KEPT (the canonical 60/15 windows overlap by exactly 25%, well under this).
const OVERLAP_FRACTION: f64 = 0.5;

/// Stage-B content-similarity threshold (1c): a later hit whose `code_tokens`
/// set Jaccard to an earlier, higher-ranked kept hit is `>=` this value is
/// dropped as a cross-file near-duplicate. CONSERVATIVE on purpose — biased
/// toward KEEPING distinct hits: two files sharing only a
/// license/import prologue have divergent token sets well below this, so they
/// are never collapsed. Tunable.
const CONTENT_JACCARD_THRESHOLD: f64 = 0.9;

/// Parse a `path:start-end` chunk id into `(file_path, start, end)`.
///
/// The id format is `path:start-end` (see `chunker::chunk_id`); the path itself
/// may contain `:` on some platforms, so we split on the LAST `:` and parse the
/// trailing `start-end` range. Returns `None` for any id not matching the shape
/// (such ids are simply left untouched by dedup — fail-open, never drop).
fn parse_chunk_id(id: &str) -> Option<(&str, i64, i64)> {
    let (path, range) = id.rsplit_once(':')?;
    let (start, end) = range.split_once('-')?;
    let start: i64 = start.parse().ok()?;
    let end: i64 = end.parse().ok()?;
    Some((path, start, end))
}

/// Fraction of the SHORTER of two `[start,end]` line ranges that they overlap.
/// Both ranges are inclusive (length = end - start + 1). Returns 0.0 when they
/// do not overlap. A zero-length shorter range (malformed) yields 0.0.
fn overlap_fraction(a: (i64, i64), b: (i64, i64)) -> f64 {
    let lo = a.0.max(b.0);
    let hi = a.1.min(b.1);
    if hi < lo {
        return 0.0;
    }
    let overlap = (hi - lo + 1) as f64;
    let len_a = (a.1 - a.0 + 1).max(0) as f64;
    let len_b = (b.1 - b.0 + 1).max(0) as f64;
    let shorter = len_a.min(len_b);
    if shorter <= 0.0 {
        return 0.0;
    }
    overlap / shorter
}

/// Stage A of near-duplicate dedup (1c): same-file overlapping-range collapse,
/// operating on the fused `(chunk_id, score)` list ALONE — no DB read, the line
/// ranges are carried in the `path:start-end` ids.
///
/// The input is the output of [`rrf_fuse`], i.e. already sorted best-first.
/// For each hit, if a HIGHER-ranked already-kept hit from the SAME `file_path`
/// overlaps its range by MORE than [`OVERLAP_FRACTION`] of the shorter range,
/// the (lower-scored) later hit is dropped. Relative order of survivors is
/// preserved. Catches window-overlap and symbol-vs-module-wrapper duplicates.
///
/// Cost: O(n²) id parsing over the (small) candidate list, zero queries.
pub fn dedup_overlapping(fused: &mut Vec<(String, f64)>) {
    let mut kept: Vec<(String, f64)> = Vec::with_capacity(fused.len());
    for (id, score) in fused.drain(..) {
        let Some((path, start, end)) = parse_chunk_id(&id) else {
            // Unparseable id: keep it (fail-open, never silently drop).
            kept.push((id, score));
            continue;
        };
        let is_dup = kept.iter().any(|(kid, _)| match parse_chunk_id(kid) {
            Some((kpath, kstart, kend)) => {
                kpath == path && overlap_fraction((start, end), (kstart, kend)) > OVERLAP_FRACTION
            }
            None => false,
        });
        if !is_dup {
            kept.push((id, score));
        }
    }
    *fused = kept;
}

/// Read the stored feature-hash embedding for each `chunk_id` back out of
/// `chunks_vec` (the MMR similarity source).
///
/// We READ the persisted vector rather than recompute it from the hydrated body:
/// the bytes are already on disk as little-endian `f32`s (see
/// `storage::insert_chunk_full`), so a single indexed JOIN per id is cheaper than
/// re-tokenizing + re-hashing the body, and it guarantees byte-for-byte identity
/// with what KNN ranked. Ids with no embedding row (none expected for indexed
/// chunks) are simply omitted; MMR treats a missing vector as zero similarity.
pub fn load_embeddings(
    conn: &Connection,
    chunk_ids: &[String],
) -> Result<HashMap<String, Vec<f32>>> {
    let mut stmt = conn
        .prepare(
            "SELECT v.embedding FROM chunks_vec v \
             INNER JOIN chunks c ON c.rowid = v.rowid \
             WHERE c.id = ?1",
        )
        .context("prepare embedding lookup")?;
    let mut out = HashMap::with_capacity(chunk_ids.len());
    for id in chunk_ids {
        let bytes: Option<Vec<u8>> = stmt
            .query_row(params![id], |row| row.get::<_, Vec<u8>>(0))
            .optional_context("read chunk embedding")?;
        if let Some(bytes) = bytes {
            out.insert(id.clone(), le_bytes_to_f32(&bytes));
        }
    }
    Ok(out)
}

/// Decode a little-endian `f32` byte blob (as stored in `chunks_vec`) into a
/// `Vec<f32>`. Trailing bytes that do not complete a 4-byte lane are ignored.
fn le_bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Cosine similarity of two equal-length vectors. Returns 0.0 when either is
/// empty, the lengths differ, or either has zero norm (no positive evidence of
/// similarity → MMR sees them as maximally diverse).
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += (*x as f64) * (*y as f64);
        na += (*x as f64) * (*x as f64);
        nb += (*y as f64) * (*y as f64);
    }
    if na <= 0.0 || nb <= 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Greedy Maximal Marginal Relevance re-rank.
///
/// Given fused `(chunk_id, score)` candidates (best-first) and a map of each
/// id's feature-hash embedding (see [`load_embeddings`]), iteratively select the
/// candidate maximizing
/// `lambda * rel(c) - (1 - lambda) * max_{s in selected} cosine(c, s)`,
/// where `rel` is the min-max normalized fused score over the candidate set and
/// `cosine` is over the stored feature-hash vectors. The first pick (empty
/// selection set) is the most relevant candidate; subsequent picks trade
/// relevance for diversity. Scores in the returned list are the ORIGINAL fused
/// scores (MMR reorders, it does not rewrite scores).
///
/// `lambda = 1.0` reproduces the input relevance order (no diversity term);
/// `lambda = 0.0` is pure diversity. Pass [`MMR_LAMBDA`] for the default.
pub fn mmr_rerank(
    candidates: &[(String, f64)],
    embeddings: &HashMap<String, Vec<f32>>,
    lambda: f64,
) -> Vec<(String, f64)> {
    if candidates.len() <= 1 {
        return candidates.to_vec();
    }

    // Min-max normalize the fused scores into rel in [0,1]. A degenerate range
    // (all-equal scores) maps every candidate to rel = 1.0, so selection then
    // falls through to the diversity term + the input order tie-break.
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for (_, s) in candidates {
        min = min.min(*s);
        max = max.max(*s);
    }
    let range = max - min;
    let rel = |s: f64| -> f64 {
        if range > 0.0 {
            (s - min) / range
        } else {
            1.0
        }
    };

    let empty: Vec<f32> = Vec::new();
    let vec_of = |id: &str| -> &[f32] { embeddings.get(id).map(Vec::as_slice).unwrap_or(&empty) };

    let mut remaining: Vec<usize> = (0..candidates.len()).collect();
    let mut selected: Vec<usize> = Vec::with_capacity(candidates.len());

    while !remaining.is_empty() {
        let mut best_pos = 0usize; // position within `remaining`
        let mut best_mmr = f64::NEG_INFINITY;
        for (pos, &idx) in remaining.iter().enumerate() {
            let max_sim = selected
                .iter()
                .map(|&s| cosine(vec_of(&candidates[idx].0), vec_of(&candidates[s].0)))
                .fold(0.0f64, f64::max);
            let mmr = lambda * rel(candidates[idx].1) - (1.0 - lambda) * max_sim;
            // Strict `>` keeps the EARLIER (more-relevant, lower input index)
            // candidate on ties, since `remaining` is in input order.
            if mmr > best_mmr {
                best_mmr = mmr;
                best_pos = pos;
            }
        }
        let chosen = remaining.remove(best_pos);
        selected.push(chosen);
    }

    selected
        .into_iter()
        .map(|i| candidates[i].clone())
        .collect()
}

/// Optional structural boost.
///
/// When a query token (via [`code_tokens`]) matches a token of any file's
/// persisted `file_imports.source`, add [`STRUCTURAL_BOOST`] to every fused
/// candidate whose `file_path` is that file. Pure SQL over `file_imports`; no
/// new state. The re-sort that follows the boost reuses the SAME deterministic
/// tie-break as [`rrf_fuse_weighted`] (fused DESC, then chunk_id ASC — the
/// bm25_rank context is not available here, so the lexicographic id tie-break
/// alone keeps it stable). A no-op when the query has no import-matching tokens.
///
/// Default OFF at the call site so the RRF proof in `rrf_proof.rs` is unaffected.
pub fn apply_structural_boost(
    conn: &Connection,
    query: &str,
    fused: &mut [(String, f64)],
) -> Result<()> {
    let query_tokens: std::collections::HashSet<String> = code_tokens(query).into_iter().collect();
    if query_tokens.is_empty() || fused.is_empty() {
        return Ok(());
    }

    // Collect the set of file paths whose imports' sources tokenize to a token
    // the query also contains. One scan of file_imports; tokenization happens in
    // Rust so the camelCase/acronym rules stay identical to query-side tokens.
    let mut stmt = conn
        .prepare("SELECT DISTINCT file_path, source FROM file_imports")
        .context("prepare file_imports scan")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("execute file_imports scan")?;

    let mut boosted_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in rows {
        let (file_path, source) = r.context("read file_import row")?;
        if code_tokens(&source)
            .iter()
            .any(|t| query_tokens.contains(t))
        {
            boosted_files.insert(file_path);
        }
    }
    if boosted_files.is_empty() {
        return Ok(());
    }

    for (id, score) in fused.iter_mut() {
        if let Some((path, _, _)) = parse_chunk_id(id) {
            if boosted_files.contains(path) {
                *score += STRUCTURAL_BOOST;
            }
        }
    }

    // Re-sort after mutating scores: fused DESC, then chunk_id ASC (stable, no
    // bm25_rank context at this stage — the id tie-break keeps it deterministic).
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(())
}

/// One file-level import row, as stored in `file_imports`.
#[derive(Debug, Clone, Serialize)]
pub struct ImportRow {
    pub source: String,
    pub kind: String,
    pub line: i64,
}

/// One file-level export row, as stored in `file_exports`.
#[derive(Debug, Clone, Serialize)]
pub struct ExportRow {
    pub detail: String,
    pub line: i64,
}

/// A fully hydrated search hit: chunk location + symbol signature (if any) +
/// a bounded snippet + the file's structural imports/exports.
#[derive(Debug, Clone, Serialize)]
pub struct HydratedHit {
    pub chunk_id: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub kind: String,
    pub signature: Option<String>,
    pub snippet: String,
    pub imports: Vec<ImportRow>,
    pub exports: Vec<ExportRow>,
}

/// Hydrate a single `chunk_id` into a [`HydratedHit`], or `None` if no such
/// chunk exists.
///
/// The `symbols` LEFT JOIN keys on the symbols PK (`chunk_id`), so it yields at
/// most one row and `signature` is `None` for non-symbol chunks (window/module
/// chunks). Imports/exports are fetched per the chunk's `file_path`; empty vecs
/// when the file has none (e.g. unparsed languages — graceful degradation).
pub fn hydrate(conn: &Connection, chunk_id: &str) -> Result<Option<HydratedHit>> {
    let row = conn
        .query_row(
            "SELECT c.file_path, c.start_line, c.end_line, c.body, c.kind, s.signature \
             FROM chunks c LEFT JOIN symbols s ON s.chunk_id = c.id \
             WHERE c.id = ?1",
            params![chunk_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,         // file_path
                    row.get::<_, i64>(1)?,            // start_line
                    row.get::<_, i64>(2)?,            // end_line
                    row.get::<_, String>(3)?,         // body
                    row.get::<_, Option<String>>(4)?, // kind (NULL for legacy chunks)
                    row.get::<_, Option<String>>(5)?, // signature (NULL for non-symbol chunks)
                ))
            },
        )
        .optional_context("query chunk for hydration")?;

    let (file_path, start_line, end_line, body, kind, signature) = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let imports = load_imports(conn, &file_path)?;
    let exports = load_exports(conn, &file_path)?;

    Ok(Some(HydratedHit {
        chunk_id: chunk_id.to_string(),
        file_path,
        start_line,
        end_line,
        kind: kind.unwrap_or_default(),
        signature,
        snippet: truncate_chars(&body, SNIPPET_MAX),
        imports,
        exports,
    }))
}

/// Content fingerprint for Stage-B dedup: the SET of `code_tokens` over the
/// hit's body/snippet, unioned with the tokens of its `signature` when present.
///
/// Deliberately a token SET over a STRUCTURAL signal, NOT a raw snippet prefix:
/// two distinct files sharing a license/import prologue have divergent token
/// sets (their real bodies differ), so they never collapse — the false-positive
/// the conservative threshold + this fingerprint exist to prevent.
fn content_fingerprint(hit: &HydratedHit) -> std::collections::HashSet<String> {
    let mut set: std::collections::HashSet<String> =
        code_tokens(&hit.snippet).into_iter().collect();
    if let Some(sig) = &hit.signature {
        set.extend(code_tokens(sig));
    }
    set
}

/// Jaccard similarity of two token sets: |A∩B| / |A∪B|. Two empty sets are
/// defined as 0.0 (no positive evidence of duplication → never collapse).
fn jaccard(a: &std::collections::HashSet<String>, b: &std::collections::HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Stage B of near-duplicate dedup (1c): cross-file content collapse over the
/// ALREADY-HYDRATED top candidates, run by the server after [`hydrate`].
///
/// Each element is a `(HydratedHit, fused_score)` pair so the score travels
/// with its hit (Stage B only drops, never reorders). For each hit, if its
/// content fingerprint (see [`content_fingerprint`]) has a Jaccard `>=`
/// [`CONTENT_JACCARD_THRESHOLD`] to an earlier, higher-ranked kept hit, it is
/// dropped as a near-duplicate (vendored copy, near-identical body). Order of
/// survivors is preserved. The conservative threshold biases toward KEEPING
/// distinct hits — a legitimately distinct hit is never dropped.
///
/// Cost: no query beyond the hydrate the server already performed for the
/// returned set; O(n²) Jaccard over the small candidate list.
pub fn dedup_content(hits: &mut Vec<(HydratedHit, f64)>) {
    let mut kept: Vec<(HydratedHit, f64)> = Vec::with_capacity(hits.len());
    let mut prints: Vec<std::collections::HashSet<String>> = Vec::with_capacity(hits.len());
    for (hit, score) in hits.drain(..) {
        let fp = content_fingerprint(&hit);
        let is_dup = prints
            .iter()
            .any(|kept_fp| jaccard(&fp, kept_fp) >= CONTENT_JACCARD_THRESHOLD);
        if !is_dup {
            prints.push(fp);
            kept.push((hit, score));
        }
    }
    *hits = kept;
}

/// Fetch the `file_imports` rows for a file path.
fn load_imports(conn: &Connection, file_path: &str) -> Result<Vec<ImportRow>> {
    let mut stmt = conn
        .prepare("SELECT source, kind, line FROM file_imports WHERE file_path = ?1")
        .context("prepare file_imports statement")?;
    let rows = stmt
        .query_map(params![file_path], |row| {
            Ok(ImportRow {
                source: row.get(0)?,
                kind: row.get(1)?,
                line: row.get(2)?,
            })
        })
        .context("execute file_imports query")?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.context("read file_import row")?);
    }
    Ok(out)
}

/// Fetch the `file_exports` rows for a file path.
fn load_exports(conn: &Connection, file_path: &str) -> Result<Vec<ExportRow>> {
    let mut stmt = conn
        .prepare("SELECT detail, line FROM file_exports WHERE file_path = ?1")
        .context("prepare file_exports statement")?;
    let rows = stmt
        .query_map(params![file_path], |row| {
            Ok(ExportRow {
                detail: row.get(0)?,
                line: row.get(1)?,
            })
        })
        .context("execute file_exports query")?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.context("read file_export row")?);
    }
    Ok(out)
}

/// Truncate `s` to at most `max` chars, cutting on a char boundary so the
/// result is always valid UTF-8.
fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((byte_idx, _)) => s[..byte_idx].to_string(),
        None => s.to_string(),
    }
}

/// Small extension to turn rusqlite's `QueryReturnedNoRows` into `Ok(None)`
/// while attaching `anyhow` context to any other error.
trait OptionalContext<T> {
    fn optional_context(self, msg: &'static str) -> Result<Option<T>>;
}

impl<T> OptionalContext<T> for rusqlite::Result<T> {
    fn optional_context(self, msg: &'static str) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow::Error::new(e)).context(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{ExportKind, ExportStatement, ImportKind, ImportStatement};
    use crate::schema::migrate;
    use crate::storage::{
        insert_chunk_full, open_db, write_file_structural, IndexedChunk, SymbolData,
    };
    use tempfile::tempdir;

    /// Build a migrated, in-temp DB seeded with a Rust symbol chunk, a plain
    /// window chunk (no symbol, no structural rows), and a file's imports/exports.
    fn seed_db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("idx.sqlite")).unwrap();
        migrate(&conn).unwrap();

        // Symbol chunk: body_tokens will include "parse" and "string".
        let sym_chunk = IndexedChunk {
            id: "src/foo.rs:1-3".to_string(),
            file_path: "src/foo.rs".to_string(),
            start_line: 1,
            end_line: 3,
            body: "fn parseString(input: &str) -> String { input.to_string() }".to_string(),
        };
        let sym = SymbolData {
            name: "parseString".to_string(),
            signature: "fn parseString(input: &str) -> String".to_string(),
            kind: "function".to_string(),
            line: 1,
        };
        insert_chunk_full(&conn, &sym_chunk, "function", Some(&sym)).unwrap();

        // Structural rows for the same file.
        let imports = vec![ImportStatement::new(
            "std::collections::HashMap",
            ImportKind::Default(String::new()),
        )
        .with_line(1)];
        let exports =
            vec![ExportStatement::new(ExportKind::Named(vec!["parseString".into()])).with_line(2)];
        write_file_structural(&conn, "src/foo.rs", &imports, &exports).unwrap();

        // Window chunk: no symbol, no structural rows. Body has token "widget".
        let win_chunk = IndexedChunk {
            id: "src/win.txt:10-20".to_string(),
            file_path: "src/win.txt".to_string(),
            start_line: 10,
            end_line: 20,
            body: "plain widget text without any symbol".to_string(),
        };
        insert_chunk_full(&conn, &win_chunk, "window", None).unwrap();

        (dir, conn)
    }

    #[test]
    fn bm25_query_finds_chunk_by_token() {
        let (_dir, conn) = seed_db();

        // "parse" is a code_token of parseString, present in the symbol chunk.
        let hits = bm25_query(&conn, "parse", 10).unwrap();
        assert!(
            hits.iter().any(|(id, _)| id == "src/foo.rs:1-3"),
            "expected the parseString chunk in bm25 hits, got {hits:?}"
        );
        // Score column is present (bm25 is negative).
        assert!(hits.iter().all(|(_, score)| *score <= 0.0));
    }

    #[test]
    fn bm25_query_empty_and_punctuation_return_empty() {
        let (_dir, conn) = seed_db();

        // Empty query: no MATCH, no error.
        assert_eq!(bm25_query(&conn, "", 10).unwrap(), vec![]);
        // Punctuation-only query: code_tokens is empty, no MATCH, no error.
        assert_eq!(bm25_query(&conn, "!!! ... ,,,", 10).unwrap(), vec![]);
    }

    #[test]
    fn rrf_fuse_deterministic_ordering() {
        // Handcrafted lists. bm25: a, b, c (ranks 1,2,3). vector: c, a, d.
        let bm25 = vec![
            ("a".to_string(), -1.0),
            ("b".to_string(), -2.0),
            ("c".to_string(), -3.0),
        ];
        let vector = vec![
            KnnHit {
                chunk_id: "c".to_string(),
                distance: 0.1,
            },
            KnnHit {
                chunk_id: "a".to_string(),
                distance: 0.2,
            },
            KnnHit {
                chunk_id: "d".to_string(),
                distance: 0.3,
            },
        ];

        let fused = rrf_fuse(&bm25, &vector, RRF_K);

        // Expected scores (k=60):
        //   a: 1/61 + 1/62 = 0.016393 + 0.016129 = 0.032522
        //   c: 1/63 + 1/61 = 0.015873 + 0.016393 = 0.032266
        //   b: 1/62                                = 0.016129
        //   d:          1/63                       = 0.015873
        // Order by score DESC: a, c, b, d.
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c", "b", "d"]);

        // A chunk in both lists (a, rank1+rank2) outranks one in a single list
        // at the same rank (b, only bm25 rank2).
        let score = |id: &str| {
            fused
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, s)| *s)
                .unwrap()
        };
        assert!(score("a") > score("b"));
    }

    #[test]
    fn rrf_fuse_tie_broken_by_chunk_id() {
        // Two ids at identical rank in their sole list → identical fused score;
        // the chunk_id lexicographic tie-break orders "x" before "z".
        let bm25 = vec![("z".to_string(), -1.0)];
        let vector = vec![KnnHit {
            chunk_id: "x".to_string(),
            distance: 0.1,
        }];

        let fused = rrf_fuse(&bm25, &vector, RRF_K);
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        // Both score 1/61; bm25_rank tie-break: "z" has rank 1, "x" has sentinel
        // (absent from bm25), so "z" sorts first by the secondary key.
        assert_eq!(ids, vec!["z", "x"]);
    }

    // ---- weighted RRF ----

    /// Regression guard: `rrf_fuse_weighted` with equal weights `1.0/1.0` AND
    /// `rrf_fuse` (the delegating entry point) produce byte-identical ordering
    /// AND scores to the unweighted fuse. Captured expected order: a, c, b, d
    /// (same fixture and scores as `rrf_fuse_deterministic_ordering`).
    #[test]
    fn rrf_fuse_weighted_equal_weights_matches_current() {
        let bm25 = vec![
            ("a".to_string(), -1.0),
            ("b".to_string(), -2.0),
            ("c".to_string(), -3.0),
        ];
        let vector = vec![
            KnnHit {
                chunk_id: "c".to_string(),
                distance: 0.1,
            },
            KnnHit {
                chunk_id: "a".to_string(),
                distance: 0.2,
            },
            KnnHit {
                chunk_id: "d".to_string(),
                distance: 0.3,
            },
        ];

        let plain = rrf_fuse(&bm25, &vector, RRF_K);
        let weighted = rrf_fuse_weighted(&bm25, &vector, RRF_K, 1.0, 1.0);

        // Byte-identical ids AND scores (equal weights collapse to 1/(k+rank)).
        assert_eq!(plain, weighted);
        let ids: Vec<&str> = weighted.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c", "b", "d"], "captured unweighted order");
    }

    /// A 2:1 BM25 weight shifts a known ordering predictably. With equal weights
    /// the fixture ties (each id appears once at rank 1 in its sole list), so the
    /// chunk_id tie-break orders them `bm` before `ve`. Weighting bm25 2:1 makes
    /// the bm25-only id strictly outscore the vector-only id, so `bm` leads on
    /// the PRIMARY (score) key regardless of the tie-break.
    #[test]
    fn rrf_fuse_weighted_bias_shifts_order() {
        // Disjoint single-element lists, each id at rank 1 in its sole list →
        // identical fused score under equal weights, broken by bm25_rank.
        let bm25 = vec![("bm".to_string(), -1.0)];
        let vector = vec![KnnHit {
            chunk_id: "ve".to_string(),
            distance: 0.1,
        }];

        // Equal weights: identical score 1/61; bm25_rank tie-break puts the
        // bm25 id ("bm", rank 1) ahead of the vector-only id ("ve", sentinel).
        let equal = rrf_fuse_weighted(&bm25, &vector, RRF_K, 1.0, 1.0);
        let equal_ids: Vec<&str> = equal.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(equal_ids, vec!["bm", "ve"]);

        // Now bias the VECTOR list 2:1. The vector-only id's score becomes
        // 2/61 > 1/61, so it leads on the primary score key. This flips the
        // order relative to equal weights — a predictable, weight-driven shift.
        let biased = rrf_fuse_weighted(&bm25, &vector, RRF_K, 1.0, 2.0);
        let biased_ids: Vec<&str> = biased.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            biased_ids,
            vec!["ve", "bm"],
            "2:1 vector weight lifts the vector-only id"
        );

        // And the symmetric case: a 2:1 bm25 weight only widens the existing
        // lead, keeping `bm` first (score 2/61 vs 1/61).
        let bm_biased = rrf_fuse_weighted(&bm25, &vector, RRF_K, 2.0, 1.0);
        let bm_ids: Vec<&str> = bm_biased.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(bm_ids, vec!["bm", "ve"]);
        let bm_score = bm_biased.iter().find(|(i, _)| i == "bm").unwrap().1;
        let ve_score = bm_biased.iter().find(|(i, _)| i == "ve").unwrap().1;
        assert!(
            bm_score > ve_score,
            "2:1 bm25 weight strictly outscores the vector-only id"
        );
    }

    // ---- adaptive weight classification (AC1) ----

    /// A single snake_case identifier biases BM25-heavy (lexical lookup).
    #[test]
    fn classify_snake_case_identifier_is_bm25_heavy() {
        let (bm25, vector) = classify_query_weights("parse_config");
        assert!(
            bm25 > vector,
            "snake_case single token should up-weight BM25, got {bm25}/{vector}"
        );
    }

    /// A single camelCase identifier biases BM25-heavy.
    #[test]
    fn classify_camel_case_identifier_is_bm25_heavy() {
        let (bm25, vector) = classify_query_weights("parseConfig");
        assert!(
            bm25 > vector,
            "camelCase single token should up-weight BM25, got {bm25}/{vector}"
        );
    }

    /// A multi-word NL phrase biases vector-heavy (conceptual lookup).
    #[test]
    fn classify_multiword_phrase_is_vector_heavy() {
        let (bm25, vector) = classify_query_weights("read a file from disk into a string");
        assert!(
            vector > bm25,
            "multi-word NL phrase should up-weight vector, got {bm25}/{vector}"
        );
    }

    /// A single plain lowercase word is ambiguous → neutral (no bias either way).
    #[test]
    fn classify_single_plain_word_is_neutral() {
        let (bm25, vector) = classify_query_weights("parse");
        assert_eq!(
            (bm25, vector),
            (1.0, 1.0),
            "a bare lowercase word should not bias either arm"
        );
    }

    // ---- weight resolution precedence (AC2 / AC3) ----

    /// AC2: an explicit caller weight wins over the adaptive heuristic, per axis.
    /// Even with `adaptive = true` on an identifier query (which the heuristic
    /// would make BM25-heavy), the explicit `Some(w)` values pass through verbatim.
    #[test]
    fn resolve_weights_explicit_beats_adaptive() {
        // Identifier query that classify_query_weights would turn BM25-heavy.
        let (bm25, vector) = resolve_weights(
            Some(0.3),
            Some(0.7),
            /* adaptive */ true,
            "parse_config",
        );
        assert_eq!(
            (bm25, vector),
            (0.3, 0.7),
            "explicit weights must override the adaptive heuristic"
        );
    }

    /// AC2 (per-axis): one explicit axis + one unset axis with adaptive on — the
    /// explicit axis is verbatim, the unset axis comes from the heuristic.
    #[test]
    fn resolve_weights_explicit_one_axis_adaptive_other() {
        // "parse_config" → heuristic ADAPTIVE_BM25_HEAVY = (2.0, 1.0).
        let (bm25, vector) =
            resolve_weights(Some(0.5), None, /* adaptive */ true, "parse_config");
        assert_eq!(bm25, 0.5, "explicit bm25 axis is verbatim");
        assert_eq!(vector, 1.0, "unset vector axis comes from the heuristic");
    }

    /// AC3: adaptive OFF + no explicit weights → legacy 1.0/1.0 default, the
    /// byte-identical behavior the regression guard above pins.
    #[test]
    fn resolve_weights_default_off_is_neutral() {
        let (bm25, vector) = resolve_weights(None, None, /* adaptive */ false, "parse_config");
        assert_eq!(
            (bm25, vector),
            (1.0, 1.0),
            "adaptive off + no explicit weights must keep the legacy default"
        );
    }

    /// Adaptive ON + no explicit weights → the heuristic drives both axes.
    #[test]
    fn resolve_weights_adaptive_on_uses_heuristic() {
        let (bm25, vector) = resolve_weights(None, None, /* adaptive */ true, "parse_config");
        assert!(
            bm25 > vector,
            "adaptive on with an identifier query should be BM25-heavy, got {bm25}/{vector}"
        );
    }

    /// A negative explicit weight (which would invert the rank contribution) is
    /// clamped to 0.0, and a non-finite one is neutralized to 1.0 — the MCP
    /// boundary cannot smuggle a sort-breaking weight through.
    #[test]
    fn resolve_weights_clamps_negative_and_nonfinite() {
        let (bm25, vector) = resolve_weights(Some(-3.0), Some(f64::NAN), false, "q");
        assert_eq!(
            (bm25, vector),
            (0.0, 1.0),
            "negative → 0.0 (arm dropped), non-finite → 1.0 (neutral)"
        );
        let (b2, v2) = resolve_weights(Some(f64::INFINITY), Some(-0.0), false, "q");
        assert_eq!(
            (b2, v2),
            (1.0, 0.0),
            "inf → 1.0; -0.0 is non-negative → kept"
        );
    }

    // ---- MMR diversity ----

    #[test]
    fn mmr_demotes_near_identical_below_diverse_third() {
        // Three candidates, best-first by fused score: two near-identical chunks
        // ("a", "b" with near-parallel vectors) and a diverse third ("c") with an
        // orthogonal vector. "b" is only MARGINALLY more relevant than "c", so at
        // lambda = 0.7 the diversity penalty on "b" (high similarity to the
        // already-picked "a") outweighs its small relevance edge.
        let candidates = vec![
            ("a".to_string(), 0.90),
            ("b".to_string(), 0.55),
            ("c".to_string(), 0.50),
        ];
        let mut embeddings: HashMap<String, Vec<f32>> = HashMap::new();
        // a and b are nearly parallel (cosine ~1); c is orthogonal to both.
        embeddings.insert("a".to_string(), vec![1.0, 0.0, 0.0]);
        embeddings.insert("b".to_string(), vec![0.99, 0.01, 0.0]);
        embeddings.insert("c".to_string(), vec![0.0, 0.0, 1.0]);

        // OFF behavior (plain fused order) is the input order: a, b, c.
        let plain: Vec<&str> = candidates.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(plain, vec!["a", "b", "c"]);

        // MMR ON: after picking "a" (most relevant), the diverse "c" beats the
        // near-identical "b" despite "b"'s higher fused score, because "b"'s high
        // similarity to "a" is penalized. Expect a, c, b.
        let reranked = mmr_rerank(&candidates, &embeddings, MMR_LAMBDA);
        let ids: Vec<&str> = reranked.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["a", "c", "b"],
            "near-identical b demoted below diverse c"
        );

        // MMR preserves the original fused scores (reorders, never rewrites).
        let b_score = reranked.iter().find(|(i, _)| i == "b").unwrap().1;
        assert_eq!(b_score, 0.55);
    }

    #[test]
    fn mmr_lambda_one_preserves_relevance_order() {
        // lambda = 1.0 drops the diversity term entirely → input relevance order.
        let candidates = vec![
            ("a".to_string(), 0.90),
            ("b".to_string(), 0.85),
            ("c".to_string(), 0.50),
        ];
        let mut embeddings: HashMap<String, Vec<f32>> = HashMap::new();
        embeddings.insert("a".to_string(), vec![1.0, 0.0, 0.0]);
        embeddings.insert("b".to_string(), vec![0.99, 0.01, 0.0]);
        embeddings.insert("c".to_string(), vec![0.0, 0.0, 1.0]);

        let reranked = mmr_rerank(&candidates, &embeddings, 1.0);
        let ids: Vec<&str> = reranked.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    // ---- structural boost ----

    #[test]
    fn structural_boost_lifts_importing_file() {
        let (_dir, conn) = seed_db();
        // seed_db gives src/foo.rs a single import: source "std::collections::HashMap".
        // That source tokenizes to {std, collections, hash, map}; the query
        // "HashMap" tokenizes to {hash, map} (camelCase split), so it matches →
        // src/foo.rs chunks get boosted. (A bare "hashmap" would NOT split and so
        // would not match — the tokenizer is the contract on both sides.)

        // Two candidates: a window chunk slightly AHEAD of the foo.rs symbol chunk.
        let make = |id: &str, s: f64| (id.to_string(), s);
        let baseline = vec![
            make("src/win.txt:10-20", 0.50),
            make("src/foo.rs:1-3", 0.495),
        ];

        // Boost OFF (do not call): ordering is the plain fused order.
        let mut off = baseline.clone();
        off.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then_with(|| a.0.cmp(&b.0)));
        let off_ids: Vec<&str> = off.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(off_ids, vec!["src/win.txt:10-20", "src/foo.rs:1-3"]);

        // Boost ON with a query that matches the import source tokens "hash"/"map":
        // foo.rs gains STRUCTURAL_BOOST (0.01), lifting it from 0.495 to 0.505,
        // above the 0.50 window chunk.
        let mut on = baseline.clone();
        apply_structural_boost(&conn, "HashMap", &mut on).unwrap();
        let on_ids: Vec<&str> = on.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            on_ids,
            vec!["src/foo.rs:1-3", "src/win.txt:10-20"],
            "import-matching file's chunk lifted above the window chunk"
        );
    }

    #[test]
    fn structural_boost_no_match_is_noop() {
        let (_dir, conn) = seed_db();
        // A query with no token matching any import source → no score change,
        // ordering identical to the plain fused order.
        let mut fused = vec![
            ("src/win.txt:10-20".to_string(), 0.50),
            ("src/foo.rs:1-3".to_string(), 0.495),
        ];
        let before = fused.clone();
        apply_structural_boost(&conn, "completely unrelated zzzqqq", &mut fused).unwrap();
        assert_eq!(
            fused, before,
            "no import match → scores and order unchanged"
        );
    }

    #[test]
    fn hydrate_symbol_chunk() {
        let (_dir, conn) = seed_db();

        let hit = hydrate(&conn, "src/foo.rs:1-3").unwrap().unwrap();
        assert_eq!(hit.chunk_id, "src/foo.rs:1-3");
        assert_eq!(hit.file_path, "src/foo.rs");
        assert_eq!(hit.start_line, 1);
        assert_eq!(hit.end_line, 3);
        assert_eq!(hit.kind, "function");
        assert_eq!(
            hit.signature.as_deref(),
            Some("fn parseString(input: &str) -> String")
        );
        // Structural rows for this file are present.
        assert_eq!(hit.imports.len(), 1);
        assert_eq!(hit.imports[0].source, "std::collections::HashMap");
        assert_eq!(hit.imports[0].kind, "default");
        assert_eq!(hit.exports.len(), 1);
        assert_eq!(hit.exports[0].detail, "parseString");
        assert!(!hit.snippet.is_empty());
    }

    #[test]
    fn hydrate_window_chunk_has_no_symbol_or_structural() {
        let (_dir, conn) = seed_db();

        let hit = hydrate(&conn, "src/win.txt:10-20").unwrap().unwrap();
        assert_eq!(hit.kind, "window");
        // No symbol row → signature None.
        assert!(hit.signature.is_none());
        // No structural rows for this file → empty vecs (graceful degradation).
        assert!(hit.imports.is_empty());
        assert!(hit.exports.is_empty());
    }

    #[test]
    fn hydrate_missing_chunk_returns_none() {
        let (_dir, conn) = seed_db();
        assert!(hydrate(&conn, "does/not:exist").unwrap().is_none());
    }

    // ---- 1c near-duplicate dedup ----

    /// Build a minimal `HydratedHit` with the given file, range, signature, and
    /// body/snippet — enough for the Stage-B content tests.
    fn make_hit(
        file: &str,
        start: i64,
        end: i64,
        signature: Option<&str>,
        snippet: &str,
    ) -> HydratedHit {
        HydratedHit {
            chunk_id: format!("{file}:{start}-{end}"),
            file_path: file.to_string(),
            start_line: start,
            end_line: end,
            kind: "function".to_string(),
            signature: signature.map(str::to_string),
            snippet: snippet.to_string(),
            imports: vec![],
            exports: vec![],
        }
    }

    #[test]
    fn dedup_overlapping_same_file() {
        // Stage A, ids only: a.rs ranges 1-60 and 46-105 overlap by 15 lines
        // (46..=60) over a shorter range of 60 → 25% > 50%? No: 15/60 = 0.25.
        // The named case uses ranges chosen so the LOWER-scored hit is
        // dropped only when overlap > 50% of the shorter range. Use 1-60 and
        // 30-60: overlap 30..=60 = 31 lines, shorter = 31 lines → ~100% > 50%.
        let mut fused = vec![
            ("a.rs:1-60".to_string(), 0.9),
            ("a.rs:30-60".to_string(), 0.5),
            ("b.rs:1-60".to_string(), 0.4),
        ];
        dedup_overlapping(&mut fused);
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        // The lower-scored overlapping a.rs hit is dropped; order preserved.
        assert_eq!(ids, vec!["a.rs:1-60", "b.rs:1-60"]);
    }

    /// Alias matching the task brief's `dedup_drops_overlapping_same_file` name,
    /// asserting the canonical 60/15 windows (25% overlap, < 50%) are KEPT and a
    /// >50%-overlapping pair is dropped.
    #[test]
    fn dedup_drops_overlapping_same_file() {
        // Two 60/15 windows: 1-60 and 46-105 overlap 46..=60 = 15 lines over the
        // shorter 60-line range = 25% < 50% → BOTH kept (Stage A is precise).
        let mut windows = vec![
            ("w.rs:1-60".to_string(), 0.9),
            ("w.rs:46-105".to_string(), 0.8),
        ];
        dedup_overlapping(&mut windows);
        assert_eq!(windows.len(), 2, "25%-overlap windows must both survive");

        // A symbol 10-20 fully inside a module wrapper 1-100: overlap 11 lines
        // over the shorter 11-line range = 100% > 50% → the wrapper (lower) drops.
        let mut wrap = vec![
            ("m.rs:10-20".to_string(), 0.9),
            ("m.rs:1-100".to_string(), 0.4),
        ];
        dedup_overlapping(&mut wrap);
        let ids: Vec<&str> = wrap.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["m.rs:10-20"]);
    }

    #[test]
    fn dedup_overlap_exact_50_percent_is_kept() {
        // Exactly 50% overlap of the shorter range must be KEPT (strict `>`).
        // a.rs 1-20 (20 lines) and a.rs 11-30 (20 lines): overlap 11..=20 = 10
        // lines, shorter = 20 → exactly 0.5, not > 0.5 → both kept.
        let mut fused = vec![
            ("a.rs:1-20".to_string(), 0.9),
            ("a.rs:11-30".to_string(), 0.5),
        ];
        dedup_overlapping(&mut fused);
        assert_eq!(fused.len(), 2, "exact-50% overlap must be kept (strict >)");
    }

    #[test]
    fn dedup_keeps_distinct() {
        // Stage B: two hits from DIFFERENT files with disjoint bodies → both kept.
        let mut hits = vec![
            (
                make_hit(
                    "a.rs",
                    1,
                    5,
                    Some("fn alpha()"),
                    "fn alpha() { let total = compute_sum(values); total }",
                ),
                0.9,
            ),
            (
                make_hit(
                    "b.rs",
                    1,
                    5,
                    Some("fn beta()"),
                    "fn beta() { let widget = render_template(context); widget }",
                ),
                0.8,
            ),
        ];
        dedup_content(&mut hits);
        assert_eq!(hits.len(), 2, "distinct bodies must both survive Stage B");
    }

    #[test]
    fn dedup_keeps_distinct_shared_prologue() {
        // The anti-false-collapse guard: two DIFFERENT files sharing an identical
        // license/import header but with DISJOINT real bodies → BOTH kept. The
        // fingerprint is over the token multiset, so the shared prologue cannot
        // by itself push Jaccard over the conservative threshold.
        let prologue = "// SPDX-License-Identifier: MIT OR Apache-2.0\n\
             use std::collections::HashMap;\nuse std::sync::Arc;\nuse anyhow::Result;\n";
        let body_a = "fn parse_invoice(raw: &str) -> Invoice { \
             let amount = extract_amount(raw); Invoice { amount, currency: usd_code() } }";
        let body_b = "fn render_dashboard(state: &AppState) -> Html { \
             let widgets = collect_widgets(state); template_engine().render(widgets) }";
        let mut hits = vec![
            (
                make_hit(
                    "billing.rs",
                    1,
                    10,
                    Some("fn parse_invoice"),
                    &format!("{prologue}{body_a}"),
                ),
                0.9,
            ),
            (
                make_hit(
                    "ui.rs",
                    1,
                    10,
                    Some("fn render_dashboard"),
                    &format!("{prologue}{body_b}"),
                ),
                0.8,
            ),
        ];
        dedup_content(&mut hits);
        assert_eq!(
            hits.len(),
            2,
            "distinct files sharing only a prologue must both survive Stage B"
        );
    }

    #[test]
    fn dedup_content_drops_near_identical() {
        // A vendored copy: two files with essentially identical bodies → the
        // lower-ranked later one is dropped (proves Stage B actually fires).
        let body = "fn checksum(data: &[u8]) -> u32 { \
             let mut acc = 0u32; for b in data { acc = acc.wrapping_add(*b as u32); } acc }";
        let mut hits = vec![
            (make_hit("orig.rs", 1, 5, Some("fn checksum"), body), 0.9),
            (
                make_hit("vendor/copy.rs", 1, 5, Some("fn checksum"), body),
                0.8,
            ),
        ];
        dedup_content(&mut hits);
        let files: Vec<&str> = hits.iter().map(|(h, _)| h.file_path.as_str()).collect();
        assert_eq!(
            files,
            vec!["orig.rs"],
            "the later near-identical copy is dropped"
        );
    }
}
