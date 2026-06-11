// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! End-to-end integration tests for the v0.1 engine, driving the public API
//! (`open_db` + `migrate` + `index_repo` / `reindex`) against real fixture
//! directories. These tests exercise the whole pipeline — walk, chunk, parse,
//! embed, FTS, and the query-time bm25 / vector / RRF / hydrate path — rather
//! than any single module in isolation.
//!
//! Behavior covered here:
//!   - hybrid search over the demo repo returns the expected chunk.
//!   - hit shape + graceful degradation (parsed vs. unparsed languages).
//!   - incremental reindex (one changed file, no FTS5 contentless-delete error) vs. full reindex.
//!   - bidirectional code-token retrieval (snake_case <-> camelCase).

use apohara_indexer::{
    bm25_query, dedup_overlapping, hydrate, index_repo, migrate, open_db, reindex, rrf_fuse,
    vector_query, RRF_K,
};
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Copy the committed `examples/demo-repo` into a fresh temp dir so each test
/// indexes an isolated, mutable copy (the index DB lives in a SEPARATE temp dir
/// so it is never walked).
fn copy_demo_repo(dst: &Path) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-repo");
    for entry in fs::read_dir(&src).expect("read examples/demo-repo") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().unwrap();
            fs::copy(&path, dst.join(name)).unwrap();
        }
    }
}

/// Open a migrated DB inside `db_dir` (kept OUTSIDE any walked source root).
fn migrated_db(db_dir: &TempDir) -> Connection {
    let conn = open_db(&db_dir.path().join("idx.sqlite")).unwrap();
    migrate(&conn).unwrap();
    conn
}

/// Resolve the chunk id of the symbol named `name`, so assertions do not hard-code
/// line ranges that would break when the fixture files are edited.
fn chunk_id_for_symbol(conn: &Connection, name: &str) -> String {
    conn.query_row(
        "SELECT chunk_id FROM symbols WHERE name = ?1",
        rusqlite::params![name],
        |row| row.get(0),
    )
    .unwrap_or_else(|e| panic!("no symbol chunk named {name}: {e}"))
}

// ============================================================================
// Hybrid search over the demo repo.
// ============================================================================

#[test]
fn ac3_hybrid_search_finds_expected_chunk() {
    let src = TempDir::new().unwrap();
    let db = TempDir::new().unwrap();
    copy_demo_repo(src.path());

    let conn = migrated_db(&db);
    let report = index_repo(&conn, src.path()).unwrap();
    assert_eq!(report.files_indexed, 4, "demo repo has 4 source files");
    assert!(report.chunks > 0);

    // "reservoir sampling" is a distinctive phrase living ONLY in util.py's
    // reservoir_sample function. Python is now a parsed language, so that body
    // is its OWN symbol chunk; run the full hybrid path and assert that chunk
    // lands in the fused top-k. The chunk id is resolved from the symbol table
    // so the assertion survives fixture line edits.
    let query = "reservoir sampling";
    let bm25 = bm25_query(&conn, query, 10).unwrap();
    let vector = vector_query(&conn, query, 10).unwrap();
    let fused = rrf_fuse(&bm25, &vector, RRF_K);
    assert!(!fused.is_empty(), "fused result must be non-empty");

    let top_k = 5;
    let fused_top: Vec<&str> = fused
        .iter()
        .take(top_k)
        .map(|(id, _)| id.as_str())
        .collect();
    let target = chunk_id_for_symbol(&conn, "reservoir_sample");
    assert!(
        fused_top.contains(&target.as_str()),
        "expected {target} in fused top-{top_k}, got {fused_top:?}"
    );

    // Hydration of the winning chunk yields a real, displayable record.
    let top_id = &fused[0].0;
    let hit = hydrate(&conn, top_id).unwrap().expect("top hit hydrates");
    assert_eq!(hit.chunk_id, *top_id);
    assert!(!hit.snippet.is_empty());
    assert!(
        hit.snippet.contains("reservoir_sample"),
        "snippet should contain the matched function body, got: {}",
        hit.snippet
    );
}

// ============================================================================
// 1c: Stage-A dedup over real demo-repo fused results.
// ============================================================================

#[test]
fn dedup_no_overlapping_same_file_over_demo_repo() {
    let src = TempDir::new().unwrap();
    let db = TempDir::new().unwrap();
    copy_demo_repo(src.path());

    let conn = migrated_db(&db);
    index_repo(&conn, src.path()).unwrap();

    // A broad query that hits multiple chunks across the demo files, so the
    // fused list contains several same-file candidates for Stage A to thin.
    let query = "function value validate sample point";
    let bm25 = bm25_query(&conn, query, 20).unwrap();
    let vector = vector_query(&conn, query, 20).unwrap();
    let mut fused = rrf_fuse(&bm25, &vector, RRF_K);
    assert!(!fused.is_empty(), "fused result must be non-empty");

    dedup_overlapping(&mut fused);

    // Post-dedup invariant: no two surviving hits from the same file overlap by
    // MORE than 50% of the shorter range. Parse ids and check pairwise.
    let parsed: Vec<(String, i64, i64)> = fused
        .iter()
        .filter_map(|(id, _)| {
            let (path, range) = id.rsplit_once(':')?;
            let (s, e) = range.split_once('-')?;
            Some((path.to_string(), s.parse().ok()?, e.parse().ok()?))
        })
        .collect();
    for i in 0..parsed.len() {
        for j in (i + 1)..parsed.len() {
            let (ref pa, sa, ea) = parsed[i];
            let (ref pb, sb, eb) = parsed[j];
            if pa != pb {
                continue;
            }
            let lo = sa.max(sb);
            let hi = ea.min(eb);
            if hi < lo {
                continue;
            }
            let overlap = (hi - lo + 1) as f64;
            let shorter = ((ea - sa + 1).min(eb - sb + 1)).max(1) as f64;
            assert!(
                overlap / shorter <= 0.5,
                "{pa}:{sa}-{ea} and {pb}:{sb}-{eb} overlap > 50% after dedup"
            );
        }
    }
}

// ============================================================================
// Hit shape + graceful degradation.
// ============================================================================

#[test]
fn ac4_hit_shape_and_graceful_degradation() {
    let src = TempDir::new().unwrap();
    let db = TempDir::new().unwrap();
    copy_demo_repo(src.path());

    let conn = migrated_db(&db);
    index_repo(&conn, src.path()).unwrap();

    // --- Parsed Rust hit: a function symbol carries a signature, and its file's
    //     structural imports are recorded (graceful FULL fidelity). ---
    let rust_id = chunk_id_for_symbol(&conn, "circle_area");
    let rust_hit = hydrate(&conn, &rust_id)
        .unwrap()
        .expect("rust hit hydrates");
    assert_eq!(rust_hit.file_path, "geometry.rs");
    assert!(
        rust_hit.signature.is_some(),
        "a parsed Rust function must have a signature, got None"
    );
    assert!(
        rust_hit
            .signature
            .as_deref()
            .unwrap()
            .contains("circle_area"),
        "signature should name the function"
    );
    assert!(
        !rust_hit.imports.is_empty(),
        "geometry.rs has `use` imports, so the Rust hit's imports must be non-empty"
    );
    assert!(
        !rust_hit.exports.is_empty(),
        "geometry.rs has a `pub use` export, so exports must be non-empty"
    );

    // --- Parsed TypeScript hit: also carries a signature and non-empty exports. ---
    let ts_id = chunk_id_for_symbol(&conn, "validateEmail");
    let ts_hit = hydrate(&conn, &ts_id).unwrap().expect("ts hit hydrates");
    assert_eq!(ts_hit.file_path, "validation.ts");
    assert!(
        ts_hit.signature.is_some(),
        "TS function must have a signature"
    );
    assert!(
        !ts_hit.imports.is_empty(),
        "validation.ts imports from ./helpers"
    );
    assert!(
        !ts_hit.exports.is_empty(),
        "validation.ts has named exports"
    );

    // --- Parsed Python hit: Python is a parsed language, so a `def`
    //     is its OWN symbol chunk carrying a signature and the file's structural
    //     imports (generalized beyond Rust/TS). Python has no `export`
    //     keyword, so exports are intentionally empty. ---
    let py_id = chunk_id_for_symbol(&conn, "serialize_payload");
    let py_hit = hydrate(&conn, &py_id).unwrap().expect("py hit hydrates");
    assert_eq!(py_hit.file_path, "util.py");
    assert_eq!(py_hit.kind, "function");
    assert!(
        py_hit.signature.is_some(),
        "a parsed Python function must have a signature, got None"
    );
    assert!(
        py_hit
            .signature
            .as_deref()
            .unwrap()
            .contains("serialize_payload"),
        "signature should name the function"
    );
    assert!(
        !py_hit.imports.is_empty(),
        "util.py has `import json`/`import re`, so its imports must be non-empty"
    );
    assert_eq!(
        py_hit.exports.len(),
        0,
        "Python has no export keyword, so exports must be []"
    );

    // The parsed file still returns from BOTH retrieval paths.
    let bm25 = bm25_query(&conn, "serialize payload json", 10).unwrap();
    let vector = vector_query(&conn, "serialize payload json", 10).unwrap();
    assert!(
        bm25.iter().any(|(id, _)| id == &py_id),
        "the parsed .py chunk must be retrievable via bm25 (text path)"
    );
    assert!(
        vector.iter().any(|h| h.chunk_id == py_id),
        "the parsed .py chunk must be retrievable via vector path"
    );

    // --- Unparsed-file hit: graceful degradation. Files with an extension
    //     that no grammar recognizes (`.foo` here, after the v0.3.0 grammar
    //     expansion now covers Rust/TS/Python/Go/Bash/Java/C/Ruby) are
    //     chunked into fixed-size WINDOWS — no signature, no structural
    //     imports/exports — yet it is still indexed and hydrates (the "works on
    //     ANY repo" promise). There is no symbol to resolve by, so the chunk id
    //     is read straight from the `chunks` table by file path. ---
    //     imports/exports — yet it is still indexed and hydrates (the "works on
    //     ANY repo" promise). There is no symbol to resolve by, so the chunk id
    //     is read straight from the `chunks` table by file path. ---
    let rb_id: String = conn
        .query_row(
            "SELECT id FROM chunks WHERE id LIKE 'legacy.foo:%' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("legacy.foo must produce at least one indexed chunk");
    let rb_hit = hydrate(&conn, &rb_id)
        .unwrap()
        .expect("unparsed hit hydrates");
    assert_eq!(rb_hit.file_path, "legacy.foo");
    assert_eq!(
        rb_hit.kind, "window",
        "an unparsed .foo file must be window-chunked, got {}",
        rb_hit.kind
    );
    assert!(
        rb_hit.signature.is_none(),
        "unparsed .rb chunk must have signature == None"
    );
    assert!(
        rb_hit.imports.is_empty(),
        "unparsed .rb chunk must have imports == []"
    );
    assert!(
        rb_hit.exports.is_empty(),
        "unparsed .rb chunk must have exports == []"
    );
}

// ============================================================================
// Type symbols (struct/enum/class/interface/type) get rows.
// ============================================================================

#[test]
fn type_symbols_get_signature_rows() {
    let src = TempDir::new().unwrap();
    let db = TempDir::new().unwrap();
    copy_demo_repo(src.path());

    let conn = migrated_db(&db);
    index_repo(&conn, src.path()).unwrap();

    // geometry.rs declares `pub struct Shape`. The parser makes it its OWN symbol
    // chunk: a `symbols` row with kind="struct" and a signature naming it.
    let (kind, signature, chunk_id): (String, String, String) = conn
        .query_row(
            "SELECT kind, signature, chunk_id FROM symbols WHERE name = 'Shape'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("a `symbols` row must exist for the `Shape` struct");

    assert_eq!(kind, "struct", "the struct symbol's kind must be `struct`");
    assert!(
        signature.contains("Shape"),
        "the rendered signature must name the type, got {signature:?}"
    );
    assert_eq!(signature, "struct Shape");
    assert!(
        chunk_id.starts_with("geometry.rs:"),
        "the struct chunk id must be in geometry.rs, got {chunk_id}"
    );

    // The corresponding chunk row carries the same kind on the chunk itself.
    let chunk_kind: String = conn
        .query_row(
            "SELECT kind FROM chunks WHERE id = ?1",
            rusqlite::params![chunk_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(chunk_kind, "struct");

    // The struct chunk hydrates with a populated signature (the hit-shape
    // guarantee generalized to type symbols).
    let hit = hydrate(&conn, &chunk_id)
        .unwrap()
        .expect("struct chunk hydrates");
    assert_eq!(hit.kind, "struct");
    assert_eq!(hit.signature.as_deref(), Some("struct Shape"));
}

// ============================================================================
// Incremental reindex (pins the FTS5 contentless-delete fix).
// ============================================================================

#[test]
fn ac7_incremental_reindex_no_fts5_error() {
    let src = TempDir::new().unwrap();
    let db = TempDir::new().unwrap();
    copy_demo_repo(src.path());
    let n = 4usize; // demo repo: geometry.rs, validation.ts, util.py, legacy.foo

    let conn = migrated_db(&db);
    let full = index_repo(&conn, src.path()).unwrap();
    assert_eq!(full.files_indexed, n, "full index touches every file");
    assert!(!full.incremental, "index_repo is a full (force) reindex");

    // Mutate exactly ONE file on disk.
    fs::write(
        src.path().join("util.py"),
        "def only_one():\n    return 42\n",
    )
    .unwrap();

    // Incremental reindex. The changed file's FTS5 rows are deleted via the
    // contentless-delete path; this `.unwrap()` is the explicit pin that the
    // contentless-delete fix holds — a contentless_delete=1 FTS5 table must NOT raise here.
    let inc = reindex(&conn, src.path(), false)
        .expect("incremental reindex must not raise an FTS5 contentless-delete error");
    assert_eq!(inc.files_indexed, 1, "only the one changed file reindexed");
    assert!(inc.incremental, "force=false reports incremental=true");

    // Sanity: the changed file's new content is now what is indexed.
    let new_hit = bm25_query(&conn, "only_one", 10).unwrap();
    assert!(
        new_hit.iter().any(|(id, _)| id.starts_with("util.py:")),
        "the reindexed util.py must be searchable by its new content"
    );

    // A subsequent FORCE reindex rebuilds everything: N files, incremental=false.
    let full2 = reindex(&conn, src.path(), true).unwrap();
    assert_eq!(
        full2.files_indexed, n,
        "force reindex touches every file again"
    );
    assert!(!full2.incremental, "force=true reports incremental=false");
}

// ============================================================================
// Bidirectional code-token retrieval through the shared splitter.
// ============================================================================

#[test]
fn ac12_bidirectional_tokens() {
    let src = TempDir::new().unwrap();
    let db = TempDir::new().unwrap();

    // One chunk whose body contains the snake_case identifier `parse_string`,
    // another whose body contains the camelCase identifier `parseString`. The
    // shared `code_tokens` splitter normalizes BOTH to ["parse", "string"] on the
    // index side and the query side, so a query in either style retrieves both.
    fs::write(
        src.path().join("snake.rs"),
        "pub fn parse_string(input: &str) -> String { input.to_string() }\n",
    )
    .unwrap();
    fs::write(
        src.path().join("camel.ts"),
        "export function parseString(input: string): string { return input; }\n",
    )
    .unwrap();

    let conn = migrated_db(&db);
    index_repo(&conn, src.path()).unwrap();

    let snake_chunk = "snake.rs:1-1";
    let camel_chunk = "camel.ts:1-1";

    // Direction 1: a camelCase query retrieves the snake_case chunk.
    let by_camel: Vec<String> = bm25_query(&conn, "parseString", 10)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        by_camel.contains(&snake_chunk.to_string()),
        "query `parseString` must retrieve the `parse_string` chunk, got {by_camel:?}"
    );

    // Direction 2: a snake_case query retrieves the camelCase chunk.
    let by_snake: Vec<String> = bm25_query(&conn, "parse_string", 10)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        by_snake.contains(&camel_chunk.to_string()),
        "query `parse_string` must retrieve the `parseString` chunk, got {by_snake:?}"
    );

    // Belt-and-suspenders: both styles, both directions, reach BOTH chunks —
    // confirming the splitter is genuinely shared and symmetric.
    for q in ["parseString", "parse_string"] {
        let ids: Vec<String> = bm25_query(&conn, q, 10)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert!(
            ids.contains(&snake_chunk.to_string()) && ids.contains(&camel_chunk.to_string()),
            "query `{q}` should reach both chunks, got {ids:?}"
        );
    }
}
