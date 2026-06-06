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

/// Current schema version this binary supports. Bumped whenever the on-disk
/// shape changes in a way that requires a migration step. v0.7 introduces
/// versioning at `1` (the first VERSIONED schema): the `files` table gains a
/// `repo_id` column and a composite `PRIMARY KEY(repo_id, path)`.
pub const SCHEMA_VERSION: u32 = 1;

/// `meta` key recording the on-disk schema version (absent on pre-v0.7 indexes,
/// which are treated as legacy version 0).
pub const META_SCHEMA_VERSION: &str = "schema_version";

/// Sentinel `repo_id` written into recreated `files` rows during migration. The
/// structural recreation in [`migrate`] cannot know the repo's `root` (it takes
/// only a `Connection`), so it backfills this placeholder; the first
/// `reindex`/`index_repo` (which DOES have `root`) rewrites it to the real
/// `blake3(canonical(root))` value. A row still holding this placeholder means
/// the structural step is done but the real-id rewrite is pending — this is the
/// rows-not-yet-migrated guard, NOT a value that should ever appear at rest.
pub const MIGRATION_PLACEHOLDER_REPO_ID: &str = "<MIGRATION_PLACEHOLDER_REPO_ID>";

/// Bring an opened connection's schema up to the current engine shape.
///
/// Idempotent: safe to call on every open. Sets WAL + busy_timeout pragmas,
/// adds the `kind` column to `chunks` if absent, then creates the structural
/// and FTS tables with `IF NOT EXISTS`.
///
/// ## v0.7 schema migration (table recreation)
///
/// The single highest-risk operation in v0.7. The `files` table moves from
/// `path TEXT PRIMARY KEY` to a composite `PRIMARY KEY(repo_id, path)`. SQLite
/// cannot `ALTER TABLE ADD PRIMARY KEY`, so the change requires RECREATING the
/// table: `CREATE files_new(...)` → `INSERT … SELECT …, placeholder` →
/// `DROP files` → `RENAME files_new TO files`, all inside ONE transaction
/// (SQLite DDL is transactional). The `schema_version` stamp is written LAST
/// within the same transaction, but it is NOT the authoritative re-run sentinel:
/// idempotency is decided by STRUCTURE (does `files` already have `repo_id` AND
/// a composite PK?) plus a rows-not-yet-migrated guard. This survives a kill
/// after COMMIT but before the WAL checkpoint: re-opening re-detects "structure
/// already new" and no-ops the recreation.
///
/// A DB stamped with an UNKNOWN/NEWER `schema_version` (an older binary opening a
/// newer index) is rejected loudly via [`bail!`], mirroring the
/// [`verify_embedder_meta`] precedent — never silent.
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

    migrate_files_schema(conn).context("migrate files schema to v0.7 (composite PK)")?;

    Ok(())
}

/// Bring the `files` table up to the v0.7 shape: a `repo_id` column and a
/// composite `PRIMARY KEY(repo_id, path)`. See [`migrate`] for the rationale.
///
/// Idempotency is structural, not stamp-based:
/// - If `files` already has `repo_id` AND a composite PK, the structural step is
///   done; we only (re)write the version stamp and return — rows still holding
///   the placeholder are rewritten later by `reindex`/`index_repo`, which is the
///   intentional rows-not-yet-migrated continuation.
/// - Otherwise (legacy single-column PK, no `repo_id`), recreate the table in a
///   single transaction, backfilling [`MIGRATION_PLACEHOLDER_REPO_ID`].
///
/// The downgrade gate runs FIRST: a stamped version newer than [`SCHEMA_VERSION`]
/// is rejected loudly before any structural decision.
fn migrate_files_schema(conn: &Connection) -> Result<()> {
    // Downgrade gate: refuse an index stamped with a newer/unknown schema
    // version rather than silently mis-reading a shape we do not understand.
    if let Some(stored) = read_schema_version(conn)? {
        if stored > SCHEMA_VERSION {
            anyhow::bail!(
                "schema version mismatch: this index was written with schema_version {stored}, \
                 but this binary supports up to {SCHEMA_VERSION}. Refusing to open a newer index \
                 with an older binary — upgrade apohara-codesearch, or re-index with \
                 `reindex force=true` using this binary to rewrite the index at the supported \
                 version."
            );
        }
    }

    // Structural sentinel: the migration is "done" when `files` already carries
    // `repo_id` AND a composite primary key. This is authoritative across an
    // interrupted run (kill after COMMIT, before WAL checkpoint), where the
    // stamp alone could be misleading.
    let has_repo_id = column_exists(conn, "files", "repo_id")?;
    let composite_pk = files_pk_is_composite(conn)?;
    if has_repo_id && composite_pk {
        // Structure already migrated; just (re)stamp the version. Placeholder
        // rows, if any, are rewritten by the first reindex with a real root.
        write_schema_version(conn, SCHEMA_VERSION)?;
        return Ok(());
    }

    // Legacy shape (single-column PK, no repo_id): recreate the table. SQLite
    // DDL is transactional, so the whole recreation + version stamp commits
    // atomically; an abort leaves the old single-column `files` intact.
    conn.execute_batch(
        "BEGIN;\
         CREATE TABLE files_new (\
            path TEXT NOT NULL,\
            blake3_hash TEXT NOT NULL,\
            mtime INTEGER NOT NULL,\
            language TEXT,\
            repo_id TEXT NOT NULL,\
            PRIMARY KEY(repo_id, path)\
         );\
         INSERT INTO files_new (path, blake3_hash, mtime, language, repo_id)\
            SELECT path, blake3_hash, mtime, language, '<MIGRATION_PLACEHOLDER_REPO_ID>' FROM files;\
         DROP TABLE files;\
         ALTER TABLE files_new RENAME TO files;",
    )
    .context("recreate files table with composite PRIMARY KEY(repo_id, path)")?;

    // Version stamp LAST, inside the same transaction. The structural guard
    // above — not this stamp — is the authoritative re-run sentinel.
    let stamp = write_schema_version(conn, SCHEMA_VERSION);
    match stamp {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .context("commit files schema recreation"),
        Err(e) => {
            // Roll back so a failed stamp leaves the old single-column files
            // table intact rather than a half-migrated shape.
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// True when `files`'s primary key spans more than one column (the v0.7 composite
/// `PRIMARY KEY(repo_id, path)`), false for the legacy single-column PK.
///
/// `PRAGMA table_info(files)` yields one row per column; the `pk` field (index 5)
/// is `0` for non-PK columns and the 1-based position within the PK otherwise. A
/// composite PK therefore has two or more columns with `pk > 0`.
fn files_pk_is_composite(conn: &Connection) -> Result<bool> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(files)")
        .context("prepare table_info(files)")?;
    let mut rows = stmt.query([]).context("query table_info(files)")?;
    let mut pk_cols = 0u32;
    while let Some(row) = rows.next().context("read table_info(files) row")? {
        let pk: i64 = row.get(5).context("read pk field")?;
        if pk > 0 {
            pk_cols += 1;
        }
    }
    Ok(pk_cols >= 2)
}

/// Read the stored `schema_version`, or `None` when absent (a pre-v0.7 / legacy
/// index, treated as version 0).
fn read_schema_version(conn: &Connection) -> Result<Option<u32>> {
    match read_meta(conn, META_SCHEMA_VERSION)? {
        Some(s) => {
            let v: u32 = s
                .parse()
                .with_context(|| format!("parse stored schema_version '{s}'"))?;
            Ok(Some(v))
        }
        None => Ok(None),
    }
}

/// Record the schema version in the `meta` table (idempotent upsert).
fn write_schema_version(conn: &Connection, version: u32) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![META_SCHEMA_VERSION, version.to_string()],
    )
    .context("write meta schema_version")?;
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

/// TEST-ONLY: build a DB with EXACTLY the pre-v0.7 (legacy) schema shape.
///
/// AC0 (blocker #1): a deterministic, in-code legacy builder — NOT a committed
/// `.db` binary blob. Given a fresh [`Connection`] (already carrying the
/// `open_db` chunks/chunks_vec tables) it creates the structural tables AS THEY
/// EXISTED before v0.7: `files(path TEXT PRIMARY KEY, blake3_hash, mtime,
/// language)` with a SINGLE-column PK and NO `repo_id`, plus chunks_fts /
/// symbols / file_imports / file_exports / meta, and NO `schema_version` meta
/// key. It then inserts a couple of known seed rows so migration/parity tests
/// have deterministic content to assert against.
///
/// `pub(crate)` + `#[cfg(test)]` so other test modules (incremental, registry)
/// can reuse it without a committed fixture file.
#[cfg(test)]
pub(crate) fn build_legacy_pre_v07_db(conn: &Connection) -> Result<()> {
    // The legacy structural schema verbatim — single-column PK files, no repo_id.
    conn.execute_batch(
        "ALTER TABLE chunks ADD COLUMN kind TEXT;\
         CREATE TABLE files (\
            path TEXT PRIMARY KEY,\
            blake3_hash TEXT NOT NULL,\
            mtime INTEGER NOT NULL,\
            language TEXT\
         );\
         CREATE TABLE symbols (\
            chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,\
            name TEXT NOT NULL,\
            signature TEXT NOT NULL,\
            kind TEXT NOT NULL,\
            line INTEGER NOT NULL,\
            PRIMARY KEY(chunk_id)\
         );\
         CREATE TABLE file_imports (\
            file_path TEXT NOT NULL,\
            source TEXT NOT NULL,\
            kind TEXT NOT NULL,\
            line INTEGER NOT NULL\
         );\
         CREATE TABLE file_exports (\
            file_path TEXT NOT NULL,\
            detail TEXT NOT NULL,\
            line INTEGER NOT NULL\
         );\
         CREATE VIRTUAL TABLE chunks_fts USING fts5(\
            body_tokens,\
            content='',\
            contentless_delete=1,\
            tokenize=\"unicode61 tokenchars '_'\"\
         );\
         CREATE TABLE meta (\
            key TEXT PRIMARY KEY,\
            value TEXT NOT NULL\
         );",
    )
    .context("build legacy pre-v0.7 schema")?;

    // Two known seed rows in `files` so parity is checkable post-migration.
    conn.execute(
        "INSERT INTO files (path, blake3_hash, mtime, language) VALUES \
         ('src/main.rs', 'hashmain', 100, 'rust'), \
         ('src/lib.rs', 'hashlib', 200, 'rust')",
        [],
    )
    .context("seed legacy files rows")?;

    Ok(())
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

    /// Count the columns flagged as part of `files`'s primary key.
    fn files_pk_col_count(conn: &Connection) -> u32 {
        let mut stmt = conn.prepare("PRAGMA table_info(files)").unwrap();
        let mut rows = stmt.query([]).unwrap();
        let mut n = 0u32;
        while let Some(row) = rows.next().unwrap() {
            let pk: i64 = row.get(5).unwrap();
            if pk > 0 {
                n += 1;
            }
        }
        n
    }

    /// AC0: the in-code legacy builder is deterministic and produces EXACTLY the
    /// pre-v0.7 shape — single-column PK files, no `repo_id`, no `schema_version`
    /// — plus the known seed rows, reproducibly across two fresh runs.
    #[test]
    fn build_legacy_pre_v07_db_is_deterministic() {
        let mk = || {
            let dir = tempdir().unwrap();
            let conn = open_db(&dir.path().join("legacy.sqlite")).unwrap();
            build_legacy_pre_v07_db(&conn).unwrap();
            // Pre-v0.7 shape assertions.
            assert!(table_exists(&conn, "files"));
            assert!(!column_exists(&conn, "files", "repo_id").unwrap());
            assert_eq!(files_pk_col_count(&conn), 1, "single-column PK");
            assert!(
                read_schema_version(&conn).unwrap().is_none(),
                "no schema_version key on a legacy DB"
            );
            // Snapshot the seed rows.
            let mut stmt = conn
                .prepare("SELECT path, blake3_hash, mtime, language FROM files ORDER BY path")
                .unwrap();
            let rows: Vec<(String, String, i64, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            (dir, rows)
        };
        let (_d1, rows1) = mk();
        let (_d2, rows2) = mk();
        assert_eq!(rows1, rows2, "legacy builder must be deterministic");
        assert_eq!(rows1.len(), 2, "two known seed rows");
        assert_eq!(rows1[0].0, "src/lib.rs");
        assert_eq!(rows1[1].0, "src/main.rs");
    }

    /// AC1: a fresh DB stamps `schema_version` and gives `files` a composite PK.
    #[test]
    fn fresh_db_stamps_version_and_has_composite_pk() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("fresh.sqlite")).unwrap();
        migrate(&conn).unwrap();

        assert_eq!(read_schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
        assert!(column_exists(&conn, "files", "repo_id").unwrap());
        assert!(files_pk_is_composite(&conn).unwrap());
        assert_eq!(files_pk_col_count(&conn), 2, "PRIMARY KEY(repo_id, path)");
    }

    /// AC2 (structural half): migrating a legacy DB recreates `files` with the
    /// composite PK, backfills the placeholder repo_id on existing rows, stamps
    /// the version, and preserves the seed rows' content (path/hash/mtime/lang).
    /// The real-id rewrite (placeholder -> blake3(root)) is covered by the
    /// incremental.rs reindex test; here we assert the migration's own output.
    #[test]
    fn legacy_db_migrates_via_recreation_with_placeholder() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("legacy.sqlite")).unwrap();
        build_legacy_pre_v07_db(&conn).unwrap();

        migrate(&conn).unwrap();

        assert!(files_pk_is_composite(&conn).unwrap());
        assert_eq!(read_schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));

        // Both legacy rows survived, carrying the migration placeholder repo_id.
        let placeholder_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE repo_id = ?1",
                rusqlite::params![MIGRATION_PLACEHOLDER_REPO_ID],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            placeholder_rows, 2,
            "both legacy rows backfilled placeholder"
        );

        // Content parity for a known row.
        let (hash, mtime, lang): (String, i64, String) = conn
            .query_row(
                "SELECT blake3_hash, mtime, language FROM files WHERE path = 'src/main.rs'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (hash.as_str(), mtime, lang.as_str()),
            ("hashmain", 100, "rust")
        );

        // Idempotent: a second migrate is a structural no-op (still composite,
        // still stamped, rows unchanged).
        migrate(&conn).unwrap();
        assert!(files_pk_is_composite(&conn).unwrap());
        let still_two: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(still_two, 2);
    }

    /// AC3 (constructed intermediate state, NOT kill-timing): hand-build a DB
    /// that is ALREADY structurally recreated (composite PK + repo_id present)
    /// but whose `schema_version` stamp is ABSENT and whose rows still hold the
    /// migration placeholder. Re-running `migrate` must detect "already migrated
    /// structurally" via the structural guard and complete as a deterministic
    /// no-further-recreation: it stamps the version and leaves the placeholder
    /// rows for `reindex` to rewrite — it does NOT recreate the table again.
    #[test]
    fn constructed_intermediate_state_completes_as_noop() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("mid.sqlite")).unwrap();
        // Build the legacy base, then manually apply ONLY the structural step
        // (composite PK, repo_id present, placeholder rows) WITHOUT the stamp.
        build_legacy_pre_v07_db(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE files_new (\
                path TEXT NOT NULL, blake3_hash TEXT NOT NULL, mtime INTEGER NOT NULL,\
                language TEXT, repo_id TEXT NOT NULL, PRIMARY KEY(repo_id, path));\
             INSERT INTO files_new (path, blake3_hash, mtime, language, repo_id)\
                SELECT path, blake3_hash, mtime, language, '<MIGRATION_PLACEHOLDER_REPO_ID>' FROM files;\
             DROP TABLE files;\
             ALTER TABLE files_new RENAME TO files;",
        )
        .unwrap();
        // Sanity: structure migrated, stamp absent, rows hold placeholder.
        assert!(files_pk_is_composite(&conn).unwrap());
        assert!(read_schema_version(&conn).unwrap().is_none());

        migrate(&conn).unwrap();

        // The stamp is now written; the structure is unchanged (no second
        // recreation), and placeholder rows are still present for reindex.
        assert_eq!(read_schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
        assert!(files_pk_is_composite(&conn).unwrap());
        let placeholder_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE repo_id = ?1",
                rusqlite::params![MIGRATION_PLACEHOLDER_REPO_ID],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(placeholder_rows, 2, "rows preserved for reindex to rewrite");
    }

    /// AC3 (aborted-transaction half): an abort during the recreation leaves the
    /// OLD single-column `files` intact — never a half-migrated shape. We
    /// simulate by attempting the recreation inside a transaction and rolling it
    /// back, then asserting the legacy shape is unchanged.
    #[test]
    fn aborted_migration_leaves_legacy_files_intact() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("abort.sqlite")).unwrap();
        build_legacy_pre_v07_db(&conn).unwrap();

        conn.execute_batch(
            "BEGIN;\
             CREATE TABLE files_new (path TEXT NOT NULL, blake3_hash TEXT NOT NULL,\
                mtime INTEGER NOT NULL, language TEXT, repo_id TEXT NOT NULL,\
                PRIMARY KEY(repo_id, path));\
             INSERT INTO files_new (path, blake3_hash, mtime, language, repo_id)\
                SELECT path, blake3_hash, mtime, language, '<MIGRATION_PLACEHOLDER_REPO_ID>' FROM files;\
             DROP TABLE files;\
             ALTER TABLE files_new RENAME TO files;\
             ROLLBACK;",
        )
        .unwrap();

        // Old shape intact: single-column PK, no repo_id, both seed rows present.
        assert!(!column_exists(&conn, "files", "repo_id").unwrap());
        assert_eq!(files_pk_col_count(&conn), 1);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2, "legacy rows untouched after rollback");
    }

    /// AC4: an older binary opening a DB stamped with a NEWER/unknown
    /// `schema_version` fails loudly with a clear reindex instruction, never
    /// silently. We forge a future version stamp, then migrate.
    #[test]
    fn downgrade_gate_bails_on_newer_schema_version() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("future.sqlite")).unwrap();
        migrate(&conn).unwrap();

        // Forge a version far in the future (an index written by a newer build).
        write_schema_version(&conn, SCHEMA_VERSION + 99).unwrap();

        let err = migrate(&conn).expect_err("newer schema_version must be refused");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("schema version mismatch"),
            "clear error: {msg}"
        );
        assert!(msg.contains("reindex"), "instructs reindex: {msg}");
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
