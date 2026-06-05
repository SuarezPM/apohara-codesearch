// SPDX-License-Identifier: MIT OR Apache-2.0

//! Idempotent schema migration run on every open AFTER [`crate::storage::open_db`].
//!
//! `open_db` only creates the legacy `chunks` + `chunks_vec` tables (no `kind`
//! column). This module brings the schema up to the v0.1 engine shape: it adds
//! the `kind` column to `chunks` (guarded), and creates the `files`, `symbols`,
//! `file_imports`, `file_exports`, and `chunks_fts` tables. Every statement is
//! `IF NOT EXISTS` / guarded so calling `migrate` repeatedly is a no-op.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Bring an opened connection's schema up to the v0.1 engine shape.
///
/// Idempotent: safe to call on every open. Sets WAL + busy_timeout pragmas,
/// adds the `kind` column to `chunks` if absent, then creates the structural
/// and FTS tables with `IF NOT EXISTS`.
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;\
         PRAGMA busy_timeout=5000;",
    )
    .context("set pragmas (WAL + busy_timeout)")?;

    // Guarded `kind` column on chunks: source open_db creates chunks WITHOUT a
    // kind column, so add it only when not already present (re-running ALTER
    // TABLE ADD COLUMN would otherwise error).
    if !column_exists(conn, "chunks", "kind")? {
        conn.execute("ALTER TABLE chunks ADD COLUMN kind TEXT;", [])
            .context("add kind column to chunks")?;
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS files (
            path TEXT PRIMARY KEY,
            blake3_hash TEXT NOT NULL,
            mtime INTEGER NOT NULL,
            language TEXT
         );
         CREATE TABLE IF NOT EXISTS symbols (
            chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            signature TEXT NOT NULL,
            kind TEXT NOT NULL,
            line INTEGER NOT NULL,
            PRIMARY KEY(chunk_id)
         );
         CREATE TABLE IF NOT EXISTS file_imports (
            file_path TEXT NOT NULL,
            source TEXT NOT NULL,
            kind TEXT NOT NULL,
            line INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_file_imports_path ON file_imports(file_path);
         CREATE TABLE IF NOT EXISTS file_exports (
            file_path TEXT NOT NULL,
            detail TEXT NOT NULL,
            line INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_file_exports_path ON file_exports(file_path);
         CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
            body_tokens,
            content='',
            contentless_delete=1,
            tokenize=\"unicode61 tokenchars '_'\"
         );
         CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
         );",
    )
    .context("create structural + fts schema")?;

    Ok(())
}

/// `meta` key for the embedder id the index was built with.
pub const META_EMBEDDER_ID: &str = "embedder_id";
/// `meta` key for the embedding dimension the index was built with.
pub const META_EMBEDDER_DIM: &str = "embedder_dim";

/// Record the active embedder's `(id, dim)` in the `meta` table.
///
/// Called once when a fresh index is first written (the [`crate::storage`] write
/// path), so the `chunks_vec` DDL width and the embedder that produced the
/// vectors are both pinned. Idempotent: re-recording the SAME `(id, dim)` is a
/// no-op; an attempt to record a DIFFERENT pair on an already-stamped index is
/// rejected by [`verify_embedder_meta`] at open/query time, never here.
pub fn write_embedder_meta(conn: &Connection, id: &str, dim: usize) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![META_EMBEDDER_ID, id],
    )
    .context("write meta embedder_id")?;
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![META_EMBEDDER_DIM, dim.to_string()],
    )
    .context("write meta embedder_dim")?;
    Ok(())
}

/// Read the stored embedder `(id, dim)` an index was built with, or `None` when
/// the index predates the meta table or was never written to (empty index).
pub fn read_embedder_meta(conn: &Connection) -> Result<Option<(String, usize)>> {
    let id: Option<String> = read_meta(conn, META_EMBEDDER_ID)?;
    let dim_s: Option<String> = read_meta(conn, META_EMBEDDER_DIM)?;
    match (id, dim_s) {
        (Some(id), Some(dim_s)) => {
            let dim: usize = dim_s
                .parse()
                .with_context(|| format!("parse stored embedder_dim '{dim_s}'"))?;
            Ok(Some((id, dim)))
        }
        _ => Ok(None),
    }
}

/// Refuse-to-mix gate.
///
/// If the index already records an embedder `(id, dim)` that differs from the
/// `active` embedder's, return a CLEAR error instead of producing garbage: the
/// `chunks_vec` DDL is `float[N]` fixed at creation, so a different-dim embedder
/// fundamentally cannot share the index, and a same-dim-but-different-id
/// embedder would silently mis-rank. An index with NO stored meta (legacy or
/// empty) is accepted — the next write stamps it via [`write_embedder_meta`].
pub fn verify_embedder_meta(conn: &Connection, active_id: &str, active_dim: usize) -> Result<()> {
    if let Some((stored_id, stored_dim)) = read_embedder_meta(conn)? {
        if stored_id != active_id || stored_dim != active_dim {
            anyhow::bail!(
                "embedder mismatch: this index was built with embedder '{stored_id}' (dim {stored_dim}), \
                 but the active embedder is '{active_id}' (dim {active_dim}). Refusing to mix \
                 incompatible embeddings — re-index with `reindex force=true` using the active \
                 embedder, or point at the matching model."
            );
        }
    }
    Ok(())
}

/// Read a single `meta` value by key, or `None` when absent.
fn read_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(anyhow::Error::new(other)).context("read meta row"),
    })
}

/// Return true when `table` already has a column named `column`.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    // PRAGMA table_info(<table>) yields one row per column; column name is field 1.
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .context("prepare table_info")?;
    let mut rows = stmt.query([]).context("query table_info")?;
    while let Some(row) = rows.next().context("read table_info row")? {
        let name: String = row.get(1).context("read column name")?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::open_db;
    use tempfile::tempdir;

    fn table_exists(conn: &Connection, name: &str) -> bool {
        // FTS5 virtual tables also register with type='table' in sqlite_master.
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE name = ?1",
            [name],
            |_| Ok(()),
        )
        .is_ok()
    }

    #[test]
    fn migrate_is_idempotent() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("idx.sqlite")).unwrap();

        migrate(&conn).expect("first migrate");
        migrate(&conn).expect("second migrate must be a no-op");

        for t in [
            "files",
            "symbols",
            "file_imports",
            "file_exports",
            "chunks_fts",
            "meta",
        ] {
            assert!(table_exists(&conn, t), "expected table {t} to exist");
        }
        // kind column added to chunks.
        assert!(column_exists(&conn, "chunks", "kind").unwrap());
    }

    #[test]
    fn embedder_meta_round_trips_and_accepts_matching() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("m.sqlite")).unwrap();
        migrate(&conn).unwrap();

        // Empty index: no meta yet → verify accepts ANY embedder (next write stamps).
        assert!(read_embedder_meta(&conn).unwrap().is_none());
        verify_embedder_meta(&conn, "feature-hash-v1", 384).expect("empty index accepts any");

        // Stamp it, then read it back.
        write_embedder_meta(&conn, "feature-hash-v1", 384).unwrap();
        assert_eq!(
            read_embedder_meta(&conn).unwrap(),
            Some(("feature-hash-v1".to_string(), 384))
        );
        // The SAME (id, dim) is accepted (idempotent open).
        verify_embedder_meta(&conn, "feature-hash-v1", 384).expect("matching embedder accepted");
    }

    #[test]
    fn embedder_meta_refuses_to_mix() {
        // Build an index recording one embedder, then simulate opening it with a
        // DIFFERENT embedder: a clear error, not a panic or a silent mismatch.
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("mix.sqlite")).unwrap();
        migrate(&conn).unwrap();
        write_embedder_meta(&conn, "feature-hash-v1", 384).unwrap();

        // Different id, same dim → refuse (would silently mis-rank).
        let err = verify_embedder_meta(&conn, "gguf:minilm", 384)
            .expect_err("different id must be refused");
        let msg = err.to_string();
        assert!(msg.contains("embedder mismatch"), "clear error: {msg}");
        assert!(msg.contains("feature-hash-v1") && msg.contains("gguf:minilm"));

        // Different dim → refuse (chunks_vec DDL width is fixed at creation).
        let err = verify_embedder_meta(&conn, "feature-hash-v1", 768)
            .expect_err("different dim must be refused");
        assert!(err.to_string().contains("dim"));
    }

    #[test]
    fn sqlite_version_supports_contentless_delete() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("v.sqlite")).unwrap();
        let version: String = conn
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .unwrap();
        assert!(
            version_ge(&version, (3, 43, 0)),
            "contentless_delete needs sqlite >= 3.43.0, got {version}"
        );
    }

    /// Compare a `MAJOR.MINOR.PATCH` version string against a tuple.
    fn version_ge(v: &str, min: (u32, u32, u32)) -> bool {
        let parts: Vec<u32> = v.split('.').filter_map(|p| p.parse().ok()).collect();
        let got = (
            *parts.first().unwrap_or(&0),
            *parts.get(1).unwrap_or(&0),
            *parts.get(2).unwrap_or(&0),
        );
        got >= min
    }
}
