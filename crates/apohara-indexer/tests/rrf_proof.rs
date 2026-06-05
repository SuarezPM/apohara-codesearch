// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! The RRF proof harness.
//!
//! The honest centerpiece of the v0.1 evaluation: a corpus + query for which
//! BM25-alone ranks the CORRECT chunk *below* a distractor, while the
//! feature-hash vector signal lifts it, so Reciprocal Rank Fusion ranks the
//! correct chunk STRICTLY ABOVE its BM25-alone position.
//!
//! ## Why this is an honest win (not a degenerate / inverted corpus)
//!
//! The feature-hash embedding is bag-of-token-hash: its cosine similarity is, in
//! effect, distinct-token-set overlap. It has NO notion of inverse document
//! frequency (IDF). FTS5 `bm25()`, by contrast, is IDF-weighted: a match on a
//! RARE corpus term dominates several matches on COMMON terms.
//!
//! We exploit exactly that difference — not a ranking trick:
//!
//!   - The CORRECT chunk genuinely shares FOUR distinct query tokens
//!     (`load`, `save`, `cache`, `queue`). Those four tokens also appear in many
//!     filler chunks, so their IDF is LOW; BM25 therefore underweights the
//!     correct chunk. The vector, being IDF-free, sees the large token overlap
//!     and ranks the correct chunk first.
//!   - The DISTRACTOR shares exactly ONE distinct query token (`bespoke`), which
//!     is RARE in the corpus (high IDF). BM25 ranks it first on that single rare
//!     match; the vector, seeing only one shared token, ranks it far lower.
//!
//! So the correct chunk really is the better semantic match (more shared query
//! tokens), the distractor really does win BM25 for a legitimate IDF reason, and
//! RRF recovers the correct chunk. The win is reproducible because `rrf_fuse`
//! breaks ties deterministically.

use apohara_indexer::{
    bm25_query, insert_chunk_full, migrate, open_db, rrf_fuse, vector_query, IndexedChunk, RRF_K,
};
use rusqlite::Connection;
use tempfile::TempDir;

/// Insert a single chunk (no symbol; kind is cosmetic for this harness).
fn add(conn: &Connection, id: &str, body: &str) {
    let chunk = IndexedChunk {
        id: id.to_string(),
        file_path: format!("{id}.rs"),
        start_line: 1,
        end_line: 2,
        body: body.to_string(),
    };
    insert_chunk_full(conn, &chunk, "function", None).unwrap();
}

/// 1-based rank of `id` within a ranked id list, or `None` if absent.
fn rank_of(ids: &[String], id: &str) -> Option<usize> {
    ids.iter().position(|x| x == id).map(|p| p + 1)
}

#[test]
fn rrf_beats_bm25_alone() {
    let db = TempDir::new().unwrap();
    let conn = open_db(&db.path().join("idx.sqlite")).unwrap();
    migrate(&conn).unwrap();

    // Query mixes four COMMON terms with one RARE term.
    let query = "load save cache queue bespoke";

    // CORRECT: matches the four common query terms (high token overlap → strong
    // vector signal), but each term is common in the corpus → low IDF → BM25
    // underweights it.
    add(
        &conn,
        "correct",
        "fn handle() { load(); save(); cache(); queue(); }",
    );

    // DISTRACTOR: matches ONLY the rare term `bespoke` (high IDF → BM25 ranks it
    // top), shares just one token with the query → weak vector signal.
    add(&conn, "distractor", "fn weird() { bespoke(); }");

    // Filler chunks repeat the four common terms, driving their IDF down so BM25
    // genuinely prefers the rare-term distractor over the multi-term correct hit.
    for i in 0..12 {
        add(
            &conn,
            &format!("filler{i}"),
            "fn busy() { load(); save(); cache(); queue(); extra(); }",
        );
    }

    // --- BM25-alone ranking ---
    let bm25 = bm25_query(&conn, query, 50).unwrap();
    let bm25_ids: Vec<String> = bm25.iter().map(|(id, _)| id.clone()).collect();

    // --- Vector ranking (feature-hash KNN) ---
    let vector = vector_query(&conn, query, 50).unwrap();

    // --- Fused ranking ---
    let fused = rrf_fuse(&bm25, &vector, RRF_K);
    let rrf_ids: Vec<String> = fused.iter().map(|(id, _)| id.clone()).collect();

    let rank_bm25_alone =
        rank_of(&bm25_ids, "correct").expect("correct chunk must appear in the bm25-alone ranking");
    let rank_rrf =
        rank_of(&rrf_ids, "correct").expect("correct chunk must appear in the fused ranking");

    // Sanity that the corpus is honest, not degenerate:
    //   - the distractor really does beat the correct chunk under BM25-alone,
    //   - the vector really does rank the correct chunk first (genuine overlap).
    let rank_distractor_bm25 =
        rank_of(&bm25_ids, "distractor").expect("distractor must appear in the bm25-alone ranking");
    assert!(
        rank_distractor_bm25 < rank_bm25_alone,
        "honesty check: the distractor must outrank the correct chunk under BM25-alone \
         (distractor rank {rank_distractor_bm25} should be < correct rank {rank_bm25_alone})"
    );
    let vec_ids: Vec<String> = vector.iter().map(|h| h.chunk_id.clone()).collect();
    assert_eq!(
        rank_of(&vec_ids, "correct"),
        Some(1),
        "honesty check: the vector signal must rank the correct chunk first (real token overlap)"
    );

    // THE PROOF: RRF lifts the correct chunk strictly above its BM25-alone rank.
    assert!(
        rank_rrf < rank_bm25_alone,
        "RRF must rank the correct chunk strictly above BM25-alone: \
         rank_rrf={rank_rrf} should be < rank_bm25_alone={rank_bm25_alone}"
    );
}
