//! Verifies sqlite-vec chunks persist across db close + reopen (G8.A.7).
//!
//! Replaces the redb-based indexer_persistence.rs (now deleted) — same
//! property, new storage backend.

use apohara_indexer::{insert_chunk, knn_query, open_db, IndexedChunk};
use tempfile::tempdir;

#[test]
fn data_persists_across_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("idx.sqlite");

    // First session: insert chunk, then close (conn drops at end of block).
    {
        let conn = open_db(&db_path).unwrap();
        insert_chunk(
            &conn,
            &IndexedChunk {
                id: "x".into(),
                file_path: "x.rs".into(),
                start_line: 1,
                end_line: 5,
                body: "pub fn x() {}".into(),
            },
        )
        .unwrap();
    }

    // Second session: reopen + query. Chunk must still be retrievable.
    let conn = open_db(&db_path).unwrap();
    let hits = knn_query(&conn, "pub fn x", 1).unwrap();
    assert_eq!(hits.len(), 1, "data must survive db close/reopen");
    assert_eq!(hits[0].chunk_id, "x");
}

#[test]
fn empty_db_query_returns_no_hits() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("empty.sqlite");

    let conn = open_db(&db_path).unwrap();
    let hits = knn_query(&conn, "anything", 5).unwrap();
    assert!(
        hits.is_empty(),
        "knn_query on empty index must return no hits"
    );
}
