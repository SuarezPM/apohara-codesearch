// SPDX-License-Identifier: MIT OR Apache-2.0

//! Incremental (and full) reindex engine.
//!
//! [`reindex`] is the single entry point used by the MCP server. It walks the
//! repo via [`walk_repo`], then either wipes-and-rebuilds everything (`force`)
//! or applies a content-hash diff against the `files` table (incremental).
//!
//! ## Authoritative change detection
//!
//! `blake3::hash(content)` is the source of truth. `files.mtime` is only a cheap
//! pre-filter: when the stored mtime equals the file's current mtime we MAY skip
//! hashing, but a hash mismatch always reindexes — we never trust mtime alone to
//! declare a file unchanged.
//!
//! ## ROWID stability
//!
//! `chunks_vec` and `chunks_fts` are rowid-bound virtual tables whose rows are
//! joined back to `chunks` by `rowid`. SQLite reuses freed rowids, so reindexing
//! a file by `INSERT OR REPLACE` would leave the virtual-table rows pointing at a
//! *different* chunk than the one re-inserted under the same rowid. To prevent
//! that misbinding, every per-file reprocess runs in ONE transaction that DELETEs
//! the virtual-table rows (by the old chunk rowids) BEFORE deleting the `chunks`
//! rows, then re-inserts via [`insert_chunk_full`] only.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::chunker::{chunk_file, chunk_id};
use crate::embedder::{active_embedder, Embedder};
use crate::parser::{parse_source_imports_exports, FunctionSignature, Language};
use crate::schema::{verify_embedder_meta, write_embedder_meta, MIGRATION_PLACEHOLDER_REPO_ID};
use crate::storage::{
    insert_chunk_full_with, write_file_structural, IndexedChunk, SymbolData, EMBED_DIM,
};
use crate::walker::{walk_repo, WalkedFile};

/// Outcome of a [`reindex`] run, returned to the caller (and the MCP server).
#[derive(Debug, Clone, Serialize)]
pub struct ReindexReport {
    /// Number of files (re)indexed this run. For a full reindex this is every
    /// walked file; for an incremental run it is only the changed/new ones.
    pub files_indexed: usize,
    /// Total chunks written across all (re)indexed files this run.
    pub chunks: usize,
    /// Wall-clock duration of the reindex in milliseconds.
    pub duration_ms: u128,
    /// `false` for a full (force) reindex, `true` for an incremental one.
    pub incremental: bool,
}

/// Full reindex convenience wrapper (used by the MCP server's lazy first-index).
/// Equivalent to `reindex(conn, root, true)`.
///
/// Resolves [`active_embedder`] ONCE and delegates to [`index_repo_with`]. Hot
/// paths that already hold a resolved embedder (the MCP server, watch) must call
/// [`index_repo_with`] directly to avoid loading a real model twice.
pub fn index_repo(conn: &Connection, root: &Path) -> Result<ReindexReport> {
    let embedder = active_embedder(EMBED_DIM);
    index_repo_with(conn, root, embedder.as_ref())
}

/// Full reindex with a caller-supplied `embedder`. Equivalent to
/// `reindex_with(conn, root, true, embedder)`.
pub fn index_repo_with(
    conn: &Connection,
    root: &Path,
    embedder: &dyn Embedder,
) -> Result<ReindexReport> {
    reindex_with(conn, root, true, embedder)
}

/// (Re)index `root` into the already-migrated `conn`.
///
/// Thin wrapper that resolves [`active_embedder`] ONCE and delegates to
/// [`reindex_with`]. Hot paths that already hold a resolved embedder (and use its
/// `.dim()` to open the DB) must call [`reindex_with`] directly so a real model
/// is never loaded twice for one operation.
///
/// - `force == true`: delete every row from all index tables in one transaction,
///   then fully (re)index every walked file. `incremental = false`.
/// - `force == false`: hash each walked file and reprocess only changed/new
///   files; remove rows for files that vanished from disk. `incremental = true`.
pub fn reindex(conn: &Connection, root: &Path, force: bool) -> Result<ReindexReport> {
    let embedder = active_embedder(EMBED_DIM);
    reindex_with(conn, root, force, embedder.as_ref())
}

/// (Re)index `root` with a caller-supplied `embedder`.
///
/// The embedder is resolved by the CALLER (once per operation) and threaded in,
/// so a hot path that already needs the embedder's `.dim()` to open the DB does
/// NOT re-resolve it here — avoiding a duplicate model load with `gguf-embed`.
/// Behaviour is otherwise identical to [`reindex`].
pub fn reindex_with(
    conn: &Connection,
    root: &Path,
    force: bool,
    embedder: &dyn Embedder,
) -> Result<ReindexReport> {
    let start = Instant::now();
    let walked = walk_repo(root);
    let repo_id = repo_id_for(root);

    // Rewrite any placeholder repo_id rows left by the schema migration to this
    // repo's REAL id. `migrate` recreates `files` with a placeholder because it
    // does not know `root`; this is the first call that does, so it completes
    // the migration's real-id rewrite. Idempotent: a no-op once all rows hold
    // the real id (the placeholder only ever appears immediately post-migration).
    rewrite_placeholder_repo_id(conn, &repo_id)?;

    // The active embedder is supplied by the caller (resolved once per
    // operation). With no `gguf-embed` feature / no model configured this is the
    // feature-hash embedder (the no-model default). A `force` reindex re-stamps
    // the index meta with this embedder; an incremental reindex must NOT mix
    // embedders, so it refuses if the stored meta disagrees with this one.
    if !force {
        verify_embedder_meta(conn, embedder.id(), embedder.dim())
            .context("verify embedder matches index")?;
        // First incremental run on a never-stamped (empty/legacy) index: stamp it
        // now so subsequent runs can refuse to mix a different embedder.
        if crate::schema::read_embedder_meta(conn)?.is_none() {
            write_embedder_meta(conn, embedder.id(), embedder.dim())
                .context("stamp embedder meta")?;
        }
    }

    let mut files_indexed = 0usize;
    let mut chunks = 0usize;

    if force {
        wipe_all(conn)?;
        // Stamp the index with the active embedder's (id, dim) so later
        // opens/queries can refuse to mix incompatible embeddings.
        write_embedder_meta(conn, embedder.id(), embedder.dim()).context("stamp embedder meta")?;
        for file in &walked {
            let written = reprocess_file(conn, root, file, &repo_id, embedder)?;
            files_indexed += 1;
            chunks += written;
        }
    } else {
        // Drop deleted files first: anything in `files` but not walked this run.
        let walked_paths: std::collections::HashSet<&str> =
            walked.iter().map(|f| f.rel_path.as_str()).collect();
        for rel in stored_file_paths(conn, &repo_id)? {
            if !walked_paths.contains(rel.as_str()) {
                remove_file(conn, &rel, &repo_id)?;
            }
        }

        for file in &walked {
            let hash = blake3::hash(file.content.as_bytes()).to_hex().to_string();
            let mtime = file_mtime(root, &file.rel_path);
            match stored_file_state(conn, &file.rel_path, &repo_id)? {
                // Stored mtime matches and hash matches → unchanged, skip.
                // The hash check is authoritative; mtime alone never skips.
                Some((stored_hash, _stored_mtime)) if stored_hash == hash => continue,
                _ => {
                    let written =
                        reprocess_file_with(conn, file, &hash, mtime, &repo_id, embedder)?;
                    files_indexed += 1;
                    chunks += written;
                }
            }
        }
    }

    Ok(ReindexReport {
        files_indexed,
        chunks,
        duration_ms: start.elapsed().as_millis(),
        incremental: !force,
    })
}

/// Delete every row from all index tables in a single transaction. Tables are
/// cleared rather than dropped so the schema (and its pragmas) stay intact.
/// Virtual tables (`chunks_vec`, `chunks_fts`) are cleared before `chunks`.
fn wipe_all(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "BEGIN;\
         DELETE FROM chunks_vec;\
         DELETE FROM chunks_fts;\
         DELETE FROM symbols;\
         DELETE FROM file_imports;\
         DELETE FROM file_exports;\
         DELETE FROM chunks;\
         DELETE FROM files;\
         COMMIT;",
    )
    .context("wipe all index tables")
}

/// Stable, collision-resistant id for the repo rooted at `root`:
/// `blake3(canonical(root)).to_hex()`. The path is canonicalized first so the
/// same repo addressed by different spellings (relative, symlinked) maps to one
/// id — mirroring how `build_lock_for` canonicalizes (`server.rs:108`). Falls
/// back to the raw path when canonicalization fails (e.g. the dir does not yet
/// exist), so the id is still deterministic for that spelling.
fn repo_id_for(root: &Path) -> String {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    blake3::hash(canonical.to_string_lossy().as_bytes())
        .to_hex()
        .to_string()
}

/// Rewrite migration-placeholder `repo_id` rows to this repo's real `repo_id`.
///
/// `migrate` recreates `files` with [`MIGRATION_PLACEHOLDER_REPO_ID`] because it
/// cannot know `root`. The first `reindex`/`index_repo` (which has `root`)
/// completes that rewrite here. Idempotent: once no placeholder rows remain this
/// updates zero rows. Under Decision E1 (one repo per DB) every placeholder row
/// in this DB belongs to THIS repo, so the unconditional rewrite is correct.
fn rewrite_placeholder_repo_id(conn: &Connection, repo_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE files SET repo_id = ?1 WHERE repo_id = ?2",
        params![repo_id, MIGRATION_PLACEHOLDER_REPO_ID],
    )
    .context("rewrite placeholder repo_id to real id")?;
    Ok(())
}

/// Reprocess a single file for a FULL reindex: compute its hash + mtime, then
/// delegate to [`reprocess_file_with`].
fn reprocess_file(
    conn: &Connection,
    root: &Path,
    file: &WalkedFile,
    repo_id: &str,
    embedder: &dyn Embedder,
) -> Result<usize> {
    let hash = blake3::hash(file.content.as_bytes()).to_hex().to_string();
    let mtime = file_mtime(root, &file.rel_path);
    reprocess_file_with(conn, file, &hash, mtime, repo_id, embedder)
}

/// Reprocess one file in ONE transaction: delete its prior rows (virtual
/// tables first, by old chunk rowids), insert fresh chunks via
/// [`insert_chunk_full`] only, write structural rows, then upsert the `files`
/// bookkeeping row. Returns the number of chunks written.
fn reprocess_file_with(
    conn: &Connection,
    file: &WalkedFile,
    hash: &str,
    mtime: i64,
    repo_id: &str,
    embedder: &dyn Embedder,
) -> Result<usize> {
    let rel = file.rel_path.as_str();
    let specs = chunk_file(rel, &file.content, file.language.clone());

    conn.execute_batch("BEGIN").context("begin per-file txn")?;

    let result = (|| -> Result<usize> {
        // Clear rowid-bound virtual tables FIRST, keyed on the OLD chunk rowids,
        // before the chunks rows that own those rowids disappear.
        conn.execute(
            "DELETE FROM chunks_vec WHERE rowid IN \
             (SELECT rowid FROM chunks WHERE file_path = ?1)",
            params![rel],
        )
        .context("delete old chunks_vec rows")?;
        conn.execute(
            "DELETE FROM chunks_fts WHERE rowid IN \
             (SELECT rowid FROM chunks WHERE file_path = ?1)",
            params![rel],
        )
        .context("delete old chunks_fts rows")?;
        conn.execute(
            "DELETE FROM symbols WHERE chunk_id IN \
             (SELECT id FROM chunks WHERE file_path = ?1)",
            params![rel],
        )
        .context("delete old symbol rows")?;
        conn.execute(
            "DELETE FROM file_imports WHERE file_path = ?1",
            params![rel],
        )
        .context("delete old file_imports rows")?;
        conn.execute(
            "DELETE FROM file_exports WHERE file_path = ?1",
            params![rel],
        )
        .context("delete old file_exports rows")?;
        conn.execute("DELETE FROM chunks WHERE file_path = ?1", params![rel])
            .context("delete old chunk rows")?;

        // Insert the fresh chunks (sole write path: insert_chunk_full only).
        for spec in &specs {
            let id = chunk_id(rel, spec.start_line, spec.end_line);
            let indexed = IndexedChunk {
                id,
                file_path: rel.to_string(),
                start_line: spec.start_line as u32,
                end_line: spec.end_line as u32,
                body: spec.body.clone(),
            };
            let symbol = spec
                .symbol
                .as_ref()
                .map(|sig| symbol_from_signature(sig, spec.kind_str()));
            insert_chunk_full_with(conn, &indexed, spec.kind_str(), symbol.as_ref(), embedder)
                .context("insert chunk")?;
        }

        // Structural rows: only for parsed languages (Rust/TS). Unparsed files
        // produce no imports/exports.
        if let Some(lang) = file.language.clone() {
            let (imports, exports) = parse_source_imports_exports(&file.content, lang)
                .map_err(|e| anyhow::anyhow!("parse imports/exports: {e}"))?;
            write_file_structural(conn, rel, &imports, &exports)
                .context("write structural rows")?;
        }

        // Bookkeeping: upsert the files row with the authoritative hash + mtime
        // and the repo's REAL id (never a literal/placeholder). Under the
        // composite PK(repo_id, path), this row is unique per (repo, path).
        conn.execute(
            "INSERT OR REPLACE INTO files (path, blake3_hash, mtime, language, repo_id) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![rel, hash, mtime, language_tag(&file.language), repo_id],
        )
        .context("upsert files row")?;

        Ok(specs.len())
    })();

    match result {
        Ok(written) => {
            conn.execute_batch("COMMIT")
                .context("commit per-file txn")?;
            Ok(written)
        }
        Err(e) => {
            // Roll back so a failing file leaves no partial state behind.
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Remove every index row for a file that vanished from disk, in one
/// transaction. Mirror of the delete half of [`reprocess_file_with`] plus the
/// `files` row itself.
///
/// `repo_id` scopes the `files` delete as defense-in-depth (Decision E1: under
/// one-DB-per-repo `path` alone is already unique, so this predicate is
/// belt-and-suspenders, not a correctness gate).
fn remove_file(conn: &Connection, rel: &str, repo_id: &str) -> Result<()> {
    conn.execute_batch("BEGIN").context("begin remove txn")?;
    let result = (|| -> Result<()> {
        conn.execute(
            "DELETE FROM chunks_vec WHERE rowid IN \
             (SELECT rowid FROM chunks WHERE file_path = ?1)",
            params![rel],
        )?;
        conn.execute(
            "DELETE FROM chunks_fts WHERE rowid IN \
             (SELECT rowid FROM chunks WHERE file_path = ?1)",
            params![rel],
        )?;
        conn.execute(
            "DELETE FROM symbols WHERE chunk_id IN \
             (SELECT id FROM chunks WHERE file_path = ?1)",
            params![rel],
        )?;
        conn.execute(
            "DELETE FROM file_imports WHERE file_path = ?1",
            params![rel],
        )?;
        conn.execute(
            "DELETE FROM file_exports WHERE file_path = ?1",
            params![rel],
        )?;
        conn.execute("DELETE FROM chunks WHERE file_path = ?1", params![rel])?;
        conn.execute(
            "DELETE FROM files WHERE path = ?1 AND repo_id = ?2",
            params![rel, repo_id],
        )?;
        Ok(())
    })();

    match result {
        Ok(()) => conn.execute_batch("COMMIT").context("commit remove txn"),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e).context("remove deleted file")
        }
    }
}

/// All `path`s currently recorded in the `files` table for `repo_id`. The
/// `repo_id` predicate is defense-in-depth under Decision E1 (one DB per repo).
fn stored_file_paths(conn: &Connection, repo_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT path FROM files WHERE repo_id = ?1")
        .context("prepare files path query")?;
    let rows = stmt
        .query_map(params![repo_id], |row| row.get::<_, String>(0))
        .context("query files paths")?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.context("read files path row")?);
    }
    Ok(out)
}

/// The stored `(blake3_hash, mtime)` for `(repo_id, rel)`, or `None` if not yet
/// indexed. The `repo_id` predicate is defense-in-depth under Decision E1.
fn stored_file_state(conn: &Connection, rel: &str, repo_id: &str) -> Result<Option<(String, i64)>> {
    let row = conn
        .query_row(
            "SELECT blake3_hash, mtime FROM files WHERE path = ?1 AND repo_id = ?2",
            params![rel, repo_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .ok();
    Ok(row)
}

/// Read a file's mtime as whole seconds since the Unix epoch, or `0` when the
/// metadata is unavailable. mtime is only a pre-filter, so a `0` fallback simply
/// forces the authoritative hash check to decide.
fn file_mtime(root: &Path, rel: &str) -> i64 {
    let path = root.join(rel);
    std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Stable lowercase tag for a file's language, stored in `files.language`.
/// `None` for unparsed files.
fn language_tag(language: &Option<Language>) -> Option<&'static str> {
    language.as_ref().map(|l| match l {
        Language::Rust => "rust",
        Language::TypeScript => "typescript",
        Language::Python => "python",
        Language::Go => "go",
        Language::Bash => "bash",
    })
}

/// Build a [`SymbolData`] from a parsed [`FunctionSignature`], rendering a
/// single-line signature string. `kind` is the chunk's `kind_str()`
/// (`"function"`/`"method"` for callables; `"struct"`/`"enum"`/`"trait"`/
/// `"class"`/`"interface"`/`"type"` for type declarations); `line` is the
/// signature's 1-based line.
fn symbol_from_signature(sig: &FunctionSignature, kind: &str) -> SymbolData {
    SymbolData {
        name: sig.name.clone(),
        signature: render_signature(sig, kind),
        kind: kind.to_string(),
        line: sig.line as i64,
    }
}

/// Render a symbol's single-line signature.
///
/// For a CALLABLE (`kind` `"function"`/`"method"`): `name(p1: T1, p2: T2) -> Ret`
/// — parameters without a type annotation render as just their name; an absent
/// return type drops the `-> Ret` suffix.
///
/// For a TYPE declaration (any other `kind`, e.g. `"struct"`/`"class"`): just
/// `"{kind} {name}"` (e.g. `"struct Foo"`, `"class Ledger"`, `"type Handler"`).
/// Fields/generics are intentionally omitted — kind + name is enough.
fn render_signature(sig: &FunctionSignature, kind: &str) -> String {
    if matches!(kind, "function" | "method") {
        let params = sig
            .parameters
            .iter()
            .map(|p| match &p.type_annotation {
                Some(ty) => format!("{}: {}", p.name, ty),
                None => p.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        match &sig.return_type {
            Some(ret) => format!("{}({}) -> {}", sig.name, params, ret),
            None => format!("{}({})", sig.name, params),
        }
    } else {
        format!("{} {}", kind, sig.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::migrate;
    use crate::storage::open_db;
    use std::fs;
    use tempfile::TempDir;

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    /// Open a migrated DB inside a temp dir (DB lives OUTSIDE the walked root so
    /// it never gets indexed).
    fn migrated_db(db_dir: &TempDir) -> Connection {
        let conn = open_db(&db_dir.path().join("idx.sqlite")).unwrap();
        migrate(&conn).unwrap();
        conn
    }

    /// The rowid of the chunk row identified by `chunk_id`.
    fn chunk_rowid(conn: &Connection, id: &str) -> i64 {
        conn.query_row(
            "SELECT rowid FROM chunks WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn rowid_misbinding() {
        // File A and file B each carry distinct Rust functions. After a full
        // reindex we capture a KNOWN B chunk id + body, mutate A on disk, run an
        // incremental reindex, then hydrate that B chunk through BOTH its
        // chunks_vec rowid and chunks_fts rowid (joined back to chunks by rowid)
        // and assert both still resolve to B's body.
        let src = TempDir::new().unwrap();
        let db = TempDir::new().unwrap();
        let root = src.path();

        fs::write(
            root.join("a.rs"),
            "fn alpha_one() -> u32 { 1 }\n\nfn alpha_two() -> u32 { 2 }\n",
        )
        .unwrap();
        fs::write(
            root.join("b.rs"),
            "fn beta_one() -> u32 { 10 }\n\nfn beta_two() -> u32 { 20 }\n",
        )
        .unwrap();

        let conn = migrated_db(&db);
        reindex(&conn, root, true).unwrap();

        // Pick a known B symbol chunk and capture its id + body.
        let (b_id, b_body): (String, String) = conn
            .query_row(
                "SELECT c.id, c.body FROM chunks c \
                 JOIN symbols s ON s.chunk_id = c.id \
                 WHERE c.file_path = 'b.rs' AND s.name = 'beta_one'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(b_body.contains("beta_one"), "sanity: captured B body");

        // Mutate A so an incremental reindex frees + reuses A's chunk rowids.
        fs::write(
            root.join("a.rs"),
            "fn alpha_one() -> u32 { 111 }\n\nfn alpha_two() -> u32 { 222 }\n\nfn alpha_three() -> u32 { 333 }\n",
        )
        .unwrap();
        let report = reindex(&conn, root, false).unwrap();
        assert!(report.incremental);

        // The B chunk's rowid, and the bodies reachable THROUGH the virtual
        // tables by that rowid, must all still be B's body — not A's.
        let b_rowid = chunk_rowid(&conn, &b_id);

        let body_via_vec: String = conn
            .query_row(
                "SELECT c.body FROM chunks_vec v \
                 JOIN chunks c ON c.rowid = v.rowid \
                 WHERE v.rowid = ?1",
                params![b_rowid],
                |row| row.get(0),
            )
            .unwrap();
        let body_via_fts: String = conn
            .query_row(
                "SELECT c.body FROM chunks_fts f \
                 JOIN chunks c ON c.rowid = f.rowid \
                 WHERE f.rowid = ?1",
                params![b_rowid],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(body_via_vec, b_body, "chunks_vec rowid must resolve to B");
        assert_eq!(body_via_fts, b_body, "chunks_fts rowid must resolve to B");
        assert!(
            body_via_vec.contains("beta_one") && !body_via_vec.contains("alpha"),
            "B chunk must not have been overwritten by A's content"
        );
    }

    #[test]
    fn orphan() {
        // No virtual-table row may dangle without a backing chunks row.
        let src = TempDir::new().unwrap();
        let db = TempDir::new().unwrap();
        let root = src.path();

        fs::write(
            root.join("a.rs"),
            "fn alpha_one() -> u32 { 1 }\n\nfn alpha_two() -> u32 { 2 }\n",
        )
        .unwrap();
        fs::write(
            root.join("b.rs"),
            "fn beta_one() -> u32 { 10 }\n\nfn beta_two() -> u32 { 20 }\n",
        )
        .unwrap();

        let conn = migrated_db(&db);
        reindex(&conn, root, true).unwrap();

        fs::write(
            root.join("a.rs"),
            "fn alpha_one() -> u32 { 111 }\n\nfn alpha_two() -> u32 { 222 }\n\nfn alpha_three() -> u32 { 333 }\n",
        )
        .unwrap();
        reindex(&conn, root, false).unwrap();

        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM chunks_vec WHERE rowid NOT IN (SELECT rowid FROM chunks)"
            ),
            0,
            "no orphan chunks_vec rows"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM chunks_fts WHERE rowid NOT IN (SELECT rowid FROM chunks)"
            ),
            0,
            "no orphan chunks_fts rows"
        );
    }

    #[test]
    fn incremental_ac7() {
        // Index N files (force). Modify ONE. Incremental reindex touches exactly
        // 1 file and reports incremental=true, with no FTS5 error on the
        // contentless-delete path. Then a force reindex reports N files,
        // incremental=false.
        let src = TempDir::new().unwrap();
        let db = TempDir::new().unwrap();
        let root = src.path();

        let n = 4usize;
        for i in 0..n {
            fs::write(
                root.join(format!("f{i}.rs")),
                format!("fn func_{i}() -> u32 {{ {i} }}\n"),
            )
            .unwrap();
        }

        let conn = migrated_db(&db);
        let full = reindex(&conn, root, true).unwrap();
        assert_eq!(full.files_indexed, n);
        assert!(!full.incremental);

        // Modify exactly one file.
        fs::write(
            root.join("f1.rs"),
            "fn func_1() -> u32 { 999 }\n\nfn extra_1() -> u32 { 1 }\n",
        )
        .unwrap();

        // The incremental run must not raise any FTS5 error (contentless delete).
        let inc = reindex(&conn, root, false).unwrap();
        assert_eq!(inc.files_indexed, 1, "only the changed file reindexed");
        assert!(inc.incremental);

        let full2 = reindex(&conn, root, true).unwrap();
        assert_eq!(full2.files_indexed, n);
        assert!(!full2.incremental);
    }

    #[test]
    fn deleted_file_handling() {
        // Index 2 files, delete one from disk, incremental reindex → the deleted
        // file's chunks + files rows are gone.
        let src = TempDir::new().unwrap();
        let db = TempDir::new().unwrap();
        let root = src.path();

        fs::write(root.join("keep.rs"), "fn keep_me() -> u32 { 1 }\n").unwrap();
        fs::write(root.join("gone.rs"), "fn remove_me() -> u32 { 2 }\n").unwrap();

        let conn = migrated_db(&db);
        reindex(&conn, root, true).unwrap();

        assert!(
            count(
                &conn,
                "SELECT COUNT(*) FROM chunks WHERE file_path = 'gone.rs'"
            ) > 0,
            "gone.rs indexed before deletion"
        );

        fs::remove_file(root.join("gone.rs")).unwrap();
        let inc = reindex(&conn, root, false).unwrap();
        assert!(inc.incremental);

        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM chunks WHERE file_path = 'gone.rs'"
            ),
            0,
            "deleted file chunks removed"
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM files WHERE path = 'gone.rs'"),
            0,
            "deleted file bookkeeping row removed"
        );
        // The surviving file is untouched.
        assert!(
            count(
                &conn,
                "SELECT COUNT(*) FROM chunks WHERE file_path = 'keep.rs'"
            ) > 0,
            "keep.rs survives"
        );
    }

    /// Directly exercise the silent-file-drop failure mode: a file whose
    /// module-remainder splits into >= 2 Module sub-chunks must reindex WITHOUT
    /// the per-file transaction rolling back (which would drop the whole file).
    /// Then a second reindex must reproduce the same chunk set (stability).
    #[test]
    fn module_split_reindex_stable() {
        use crate::chunker::ChunkKind;

        let src = TempDir::new().unwrap();
        let db = TempDir::new().unwrap();
        let root = src.path();

        // 500 lines of top-level consts (one large uncovered run, splits into
        // several Module sub-chunks since 500 > MAX_CHUNK_LINES) plus a couple
        // of functions so the adversarial Symbol/Module adjacency is present.
        let mut body = String::new();
        body.push_str("fn lead() -> u32 { 1 }\n");
        for i in 0..500 {
            body.push_str(&format!("const ITEM_{i}: u32 = {i};\n"));
        }
        body.push_str("fn tail() -> u32 { 2 }\n");
        fs::write(root.join("big.rs"), &body).unwrap();

        // Independently chunk the file to learn the expected chunk count and a
        // known Module sub-chunk's id + body.
        let specs = chunk_file("big.rs", &body, Some(Language::Rust));
        let module_specs: Vec<_> = specs
            .iter()
            .filter(|s| s.kind == ChunkKind::Module)
            .collect();
        assert!(
            module_specs.len() >= 2,
            "fixture must split into >= 2 module sub-chunks, got {}",
            module_specs.len()
        );
        let expected_chunks = specs.len() as i64;
        let known = module_specs[1];
        let known_id = chunk_id("big.rs", known.start_line, known.end_line);
        let known_body = known.body.clone();

        let conn = migrated_db(&db);
        let report = reindex(&conn, root, true).unwrap();
        assert!(!report.incremental);

        // The per-file txn did NOT roll back: ALL of the file's chunks are
        // present (a rollback would leave 0). Count must equal what chunk_file
        // produced (> the small number of symbol chunks).
        let persisted = count(
            &conn,
            "SELECT COUNT(*) FROM chunks WHERE file_path = 'big.rs'",
        );
        assert_eq!(
            persisted, expected_chunks,
            "all chunks must persist (no per-file rollback / silent file drop)"
        );
        assert!(
            persisted > 2,
            "more than just the symbol chunks must be present"
        );

        // A known Module sub-chunk hydrates to the right body.
        let hydrated_body: String = conn
            .query_row(
                "SELECT body FROM chunks WHERE id = ?1",
                params![known_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            hydrated_body, known_body,
            "module sub-chunk body must match"
        );

        // No orphan virtual-table rows after the split insert.
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM chunks_vec WHERE rowid NOT IN (SELECT rowid FROM chunks)"
            ),
            0,
            "no orphan chunks_vec rows after module split"
        );

        // Reindex again (force) → identical chunk set, still no rollback.
        reindex(&conn, root, true).unwrap();
        let persisted2 = count(
            &conn,
            "SELECT COUNT(*) FROM chunks WHERE file_path = 'big.rs'",
        );
        assert_eq!(
            persisted2, expected_chunks,
            "chunk count stable across reindex"
        );
        let hydrated_body2: String = conn
            .query_row(
                "SELECT body FROM chunks WHERE id = ?1",
                params![known_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            hydrated_body2, known_body,
            "module sub-chunk stable across reindex"
        );
    }

    /// AC5: `repo_id_for` returns `blake3(canonical(root)).to_hex()` — the REAL
    /// hash, never a literal like `'default'` or the migration placeholder. Two
    /// distinct roots produce distinct ids; the same root is stable across calls.
    #[test]
    fn repo_id_for_is_blake3_of_canonical_root() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();

        let id_a = repo_id_for(a.path());
        let id_b = repo_id_for(b.path());

        // Real hash shape: 64 lowercase hex chars, not a literal/placeholder.
        assert_eq!(id_a.len(), 64, "blake3 hex is 64 chars");
        assert!(id_a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(id_a, "default");
        assert_ne!(id_a, MIGRATION_PLACEHOLDER_REPO_ID);

        // Distinct roots -> distinct ids; same root -> stable id.
        assert_ne!(id_a, id_b, "distinct repos get distinct ids");
        assert_eq!(id_a, repo_id_for(a.path()), "stable across calls");

        // Matches the exact construction: blake3(canonical(root)).
        let canonical = std::fs::canonicalize(a.path()).unwrap();
        let expected = blake3::hash(canonical.to_string_lossy().as_bytes())
            .to_hex()
            .to_string();
        assert_eq!(id_a, expected);
    }

    /// AC2 (real-id rewrite half): a legacy DB migrates (placeholder backfill),
    /// then the FIRST `reindex(root)` rewrites the placeholder rows to the real
    /// `blake3(canonical(root))` and indexes content. After it, NO row holds the
    /// placeholder and every files row carries the real repo_id.
    #[test]
    fn first_reindex_rewrites_placeholder_to_real_repo_id() {
        let src = TempDir::new().unwrap();
        let db = TempDir::new().unwrap();
        let root = src.path();
        fs::write(root.join("main.rs"), "fn main_one() -> u32 { 1 }\n").unwrap();

        // Build a legacy DB, migrate it (placeholder backfill happens here).
        let conn = open_db(&db.path().join("idx.sqlite")).unwrap();
        crate::schema::build_legacy_pre_v07_db(&conn).unwrap();
        migrate(&conn).unwrap();
        assert!(
            count(
                &conn,
                "SELECT COUNT(*) FROM files WHERE repo_id = '<MIGRATION_PLACEHOLDER_REPO_ID>'"
            ) > 0,
            "placeholder rows present after migrate"
        );

        // First reindex on the real root: force a full rebuild so the legacy
        // (now placeholder) rows are wiped and rewritten with the real id, and
        // any survivors are rewritten by rewrite_placeholder_repo_id.
        reindex(&conn, root, true).unwrap();

        let real_id = repo_id_for(root);
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM files WHERE repo_id = '<MIGRATION_PLACEHOLDER_REPO_ID>'"
            ),
            0,
            "no placeholder rows remain after first reindex"
        );
        let all_real: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE repo_id = ?1",
                params![real_id],
                |r| r.get(0),
            )
            .unwrap();
        let total = count(&conn, "SELECT COUNT(*) FROM files");
        assert!(total > 0, "content indexed");
        assert_eq!(all_real, total, "every files row carries the real repo_id");
    }

    /// AC2 (placeholder rewrite WITHOUT content reindex): even an INCREMENTAL
    /// reindex (no force) rewrites the migration placeholder to the real id on a
    /// row whose content has NOT changed — the bookkeeping rewrite does not
    /// require re-indexing file content.
    #[test]
    fn incremental_reindex_rewrites_placeholder_without_content_change() {
        let src = TempDir::new().unwrap();
        let db = TempDir::new().unwrap();
        let root = src.path();
        fs::write(root.join("keep.rs"), "fn keep_me() -> u32 { 7 }\n").unwrap();

        // Index once so a real files row exists, then forge the placeholder back
        // onto it to simulate the immediately-post-migration state.
        let conn = migrated_db(&db);
        reindex(&conn, root, true).unwrap();
        conn.execute(
            "UPDATE files SET repo_id = '<MIGRATION_PLACEHOLDER_REPO_ID>'",
            [],
        )
        .unwrap();

        // An incremental reindex (content unchanged → 0 files reprocessed) must
        // still rewrite the placeholder via rewrite_placeholder_repo_id.
        let report = reindex(&conn, root, false).unwrap();
        assert!(report.incremental);
        assert_eq!(
            report.files_indexed, 0,
            "content unchanged: nothing reprocessed"
        );

        let real_id = repo_id_for(root);
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM files WHERE repo_id = '<MIGRATION_PLACEHOLDER_REPO_ID>'"
            ),
            0,
            "placeholder rewritten even with no content change"
        );
        let real: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE repo_id = ?1",
                params![real_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(real > 0, "rows now carry the real id");
    }

    /// Story 2 AC3: an index built (and meta-stamped) with one embedder dim,
    /// reopened/queried with an embedder of a DIFFERENT dim, bails LOUD via
    /// `verify_embedder_meta` — never silently mixing incompatible widths.
    #[test]
    fn dim_mismatch_on_reopen_bails_loud() {
        use crate::embedder::FeatureHashEmbedder;
        use crate::schema::{verify_embedder_meta, write_embedder_meta};
        use crate::storage::{insert_chunk_full_with, open_db_with};

        let db = TempDir::new().unwrap();

        // Build a dim-768 index exactly as production does: open_db_with creates
        // the float[768] DDL, then the first write stamps the embedder's (id, dim)
        // into meta (here via the same primitives reindex uses). The 768-wide
        // feature-hash backend exercises the gate with no real model.
        let dim_x = 768usize;
        let conn = open_db_with(&db.path().join("idx.sqlite"), dim_x).unwrap();
        migrate(&conn).unwrap();
        let embedder_x = FeatureHashEmbedder::new(dim_x);
        write_embedder_meta(&conn, embedder_x.id(), embedder_x.dim()).unwrap();
        let chunk = IndexedChunk {
            id: "src/a.rs:1-1".to_string(),
            file_path: "src/a.rs".to_string(),
            start_line: 1,
            end_line: 1,
            body: "fn alpha() -> u32 { 1 }".to_string(),
        };
        insert_chunk_full_with(&conn, &chunk, "function", None, &embedder_x).unwrap();
        assert_eq!(
            crate::schema::read_embedder_meta(&conn).unwrap(),
            Some((embedder_x.id().to_string(), dim_x))
        );

        // Reopen-and-query with a DIFFERENT-dim embedder: the gate refuses.
        let embedder_y = FeatureHashEmbedder::new(384);
        let err = verify_embedder_meta(&conn, embedder_y.id(), embedder_y.dim())
            .expect_err("different dim must be refused");
        let msg = format!("{err:#}");
        assert!(msg.contains("embedder mismatch"), "clear error: {msg}");
        assert!(msg.contains("dim"), "names the dim conflict: {msg}");

        // The same-dim embedder is still accepted (idempotent reopen).
        verify_embedder_meta(&conn, embedder_x.id(), embedder_x.dim())
            .expect("matching dim accepted");
    }

    /// AC6 (multi-repo ISOLATION): two repos that BOTH contain `src/main.rs`
    /// live in SEPARATE index.db files (Decision E1). Reindexing A must not
    /// touch B's rows; B's `src/main.rs` still resolves to B's content; the two
    /// DBs carry DISTINCT repo_ids (distinct canonical roots).
    #[test]
    fn multi_repo_isolation_reindex_a_leaves_b_untouched() {
        let src_a = TempDir::new().unwrap();
        let src_b = TempDir::new().unwrap();
        let db_a = TempDir::new().unwrap();
        let db_b = TempDir::new().unwrap();

        // Same relative path, DIFFERENT content — the collision case.
        fs::create_dir_all(src_a.path().join("src")).unwrap();
        fs::create_dir_all(src_b.path().join("src")).unwrap();
        fs::write(
            src_a.path().join("src/main.rs"),
            "fn main() { alpha_marker(); }\n",
        )
        .unwrap();
        fs::write(
            src_b.path().join("src/main.rs"),
            "fn main() { beta_marker(); }\n",
        )
        .unwrap();

        let conn_a = open_db(&db_a.path().join("idx.sqlite")).unwrap();
        migrate(&conn_a).unwrap();
        let conn_b = open_db(&db_b.path().join("idx.sqlite")).unwrap();
        migrate(&conn_b).unwrap();

        reindex(&conn_a, src_a.path(), true).unwrap();
        reindex(&conn_b, src_b.path(), true).unwrap();

        // Snapshot B's bodies before re-touching A.
        let b_body_before: String = conn_b
            .query_row(
                "SELECT body FROM chunks WHERE file_path = 'src/main.rs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(b_body_before.contains("beta_marker"));

        // Reindex A again; B's DB is a different file and must be byte-unchanged.
        reindex(&conn_a, src_a.path(), true).unwrap();

        let b_body_after: String = conn_b
            .query_row(
                "SELECT body FROM chunks WHERE file_path = 'src/main.rs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(b_body_before, b_body_after, "B's content is untouched");
        assert!(b_body_after.contains("beta_marker") && !b_body_after.contains("alpha_marker"));

        // The two DBs carry distinct repo_ids (distinct canonical roots).
        let id_a: String = conn_a
            .query_row("SELECT DISTINCT repo_id FROM files", [], |r| r.get(0))
            .unwrap();
        let id_b: String = conn_b
            .query_row("SELECT DISTINCT repo_id FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(id_a, repo_id_for(src_a.path()));
        assert_eq!(id_b, repo_id_for(src_b.path()));
        assert_ne!(id_a, id_b, "distinct repos -> distinct ids");
    }
}
