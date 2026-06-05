//! Integration test for sqlite-vec storage round-trip (G8.A.2).
//!
//! This test SPECIFIES the public API that G8.A.3 must implement:
//!   - `open_db(&Path) -> Result<Connection>` (initializes schema + loads sqlite-vec ext)
//!   - `insert_chunk(&Connection, &IndexedChunk) -> Result<()>`
//!   - `knn_query(&Connection, &str, usize) -> Result<Vec<KnnHit>>`
//!
//! Until G8.A.3 lands, this test fails to compile (storage module not yet present).
//! That is the EXPECTED failing state.

use apohara_indexer::storage::{insert_chunk, knn_query, open_db, IndexedChunk};
use tempfile::tempdir;

#[test]
fn sqlite_vec_round_trip_inserts_and_retrieves_nearest() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("index.sqlite");

    let conn = open_db(&db_path).expect("open_db must initialize schema + load sqlite-vec ext");

    let chunk_a = IndexedChunk {
        id: "a".to_string(),
        file_path: "src/foo.rs".to_string(),
        start_line: 1,
        end_line: 10,
        body: "fn hello_world() {}".to_string(),
    };
    let chunk_b = IndexedChunk {
        id: "b".to_string(),
        file_path: "src/bar.rs".to_string(),
        start_line: 1,
        end_line: 10,
        body: "struct Goodbye {}".to_string(),
    };

    insert_chunk(&conn, &chunk_a).unwrap();
    insert_chunk(&conn, &chunk_b).unwrap();

    let results = knn_query(&conn, "hello world function", 1).unwrap();
    assert_eq!(
        results.len(),
        1,
        "knn_query should return exactly 1 result with k=1"
    );
    assert_eq!(
        results[0].chunk_id, "a",
        "nearest to 'hello world function' should be chunk_a (fn hello_world)"
    );
}
