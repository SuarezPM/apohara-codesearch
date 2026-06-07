// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! Robustness of the indexing pipeline against UNTRUSTED, hostile input files.
//!
//! A code-search tool is pointed at arbitrary repositories, so an indexed file
//! may be malformed, binary, or pathological. This is the end-to-end backstop for
//! the `input_validation` claim in `docs/ASSURANCE.md` § 4: a crafted repo must
//! not panic the indexer, must not error the whole run, and must leave a
//! consistent, queryable index. (The walker's own binary/minified skip is unit
//! tested in `walker.rs`; this exercises the FULL `index_repo` -> query ->
//! reindex path, including a syntactically broken file of a *parsed* language
//! that gets past the walker and into tree-sitter.)

use apohara_indexer::{bm25_query, index_repo, migrate, open_db, reindex, vector_query};
use std::fs;
use tempfile::TempDir;

#[test]
fn hostile_repo_indexes_without_panic_and_stays_queryable() {
    let src = TempDir::new().unwrap();
    let db = TempDir::new().unwrap();
    let root = src.path();

    // A valid Rust file, so the index has at least one real, queryable symbol.
    fs::write(
        root.join("valid.rs"),
        "pub fn reservoir_sample(items: &[i32], k: usize) -> Vec<i32> {\n    \
         items.iter().copied().take(k).collect()\n}\n",
    )
    .unwrap();

    // Syntactically BROKEN Rust: valid UTF-8 and short lines, so it passes the
    // walker and reaches tree-sitter, which must recover (ERROR nodes) rather
    // than panic.
    fs::write(
        root.join("broken.rs"),
        "pub fn ( { let = ; \n} impl for {{{ \nstruct Unclosed {\n  field:\n",
    )
    .unwrap();

    // Broken Python (another parsed language) — unterminated string + bad indent.
    fs::write(
        root.join("broken.py"),
        "def f(:\n    x = \"unterminated\n  return\n\tmixed_tabs_and_spaces = (\n",
    )
    .unwrap();

    // Non-UTF-8 / binary content (NUL + invalid continuation bytes). The walker
    // must skip this without erroring the run.
    fs::write(
        root.join("blob.rs"),
        [
            0x00u8, 0xff, 0xfe, 0x00, 0x80, 0x81, b'f', b'n', 0x00, 0xc3, 0x28,
        ],
    )
    .unwrap();

    // Minified-style single huge line (generated asset) — must be skipped.
    fs::write(
        root.join("min.js"),
        format!("var x=\"{}\";\n", "a".repeat(64 * 1024)),
    )
    .unwrap();

    // Empty file.
    fs::write(root.join("empty.py"), "").unwrap();

    let conn = open_db(&db.path().join("idx.sqlite")).unwrap();
    migrate(&conn).unwrap();

    // The whole hostile repo indexes without panicking and without erroring.
    let report = index_repo(&conn, root).expect("hostile repo must index, not error");
    assert!(
        report.chunks > 0,
        "the valid file should still produce indexable chunks"
    );

    // Queries run without panicking and the valid symbol is still retrievable.
    let q = "reservoir sample";
    let bm25 = bm25_query(&conn, q, 10).expect("bm25 query must not error");
    let vector = vector_query(&conn, q, 10).expect("vector query must not error");
    assert!(
        !bm25.is_empty() || !vector.is_empty(),
        "the valid symbol should be findable by at least one arm"
    );

    // Incremental reindex over the same hostile tree stays consistent (no FTS5
    // contentless-delete error, no panic).
    reindex(&conn, root, false).expect("incremental reindex over hostile repo must not error");
}
