//! sqlite-vec backed storage for code chunks + their embeddings.
//!
//! Replaces the previous Nomic BERT + redb stack (G8.A.1 dependency swap,
//! G8.A.3 implementation). Storage = sqlite-vec virtual table. Embeddings =
//! blake3 feature-hashing (see `embeddings.rs`).
//!
//! ## sqlite-vec loading
//!
//! `sqlite-vec` 0.1.9 ships only the FFI symbol `sqlite3_vec_init`. The
//! supported integration with `rusqlite` is to register it as an
//! auto-extension BEFORE opening any connection (see the upstream test
//! `sqlite-vec::tests::test_rusqlite_auto_extension`). We register once
//! per process via `OnceLock`.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

use crate::chunker::MAX_CHUNK_BYTES;
use crate::embedder::{Embedder, FeatureHashEmbedder};
use crate::embeddings::feature_hash_embed;
use crate::parser::{ExportKind, ExportStatement, ImportKind, ImportStatement};
use crate::tokens::code_tokens;

/// Truncate `s` to at most `max_bytes`, cutting on a UTF-8 char boundary so the
/// result is always valid UTF-8. Returns the whole string when already within
/// the limit.
fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Embedding dimension. Picked at 384 — wide enough for low collision rates on
/// the typical code-chunk vocabulary (a few thousand unique identifiers per
/// repo) while keeping vec0 row size at 384 * 4 = 1.5 KiB per chunk.
pub const EMBED_DIM: usize = 384;

/// One unit of indexable content: a code chunk emitted by the upstream
/// chunker (tree-sitter on the Rust side, projector rows on the TS side
/// — wired in G8.A.4 / G8.A.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedChunk {
    pub id: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub body: String,
}

/// A single KNN result. `distance` is the L2 distance reported by vec0
/// (lower = closer). Callers should treat it as opaque — only its ordering
/// across hits is meaningful.
#[derive(Debug, Clone)]
pub struct KnnHit {
    pub chunk_id: String,
    pub distance: f32,
}

/// Register sqlite-vec as an auto-extension. Safe to call repeatedly: SQLite
/// dedupes auto-extension registrations by function pointer. We still guard
/// behind `OnceLock` to avoid the FFI roundtrip on every `open_db` call.
///
/// # Process-global side effect
///
/// This installs the vec0 extension via `sqlite3_auto_extension`, which is a
/// **process-global** registration affecting every `Connection` opened
/// afterward in the same process. The `OnceLock` makes it idempotent. It is
/// `pub` so downstream crates (e.g. `apohara-episodic`) can trigger the
/// registration WITHOUT also creating the chunks schema that `open_db` builds —
/// they own their own schema but reuse this one registration primitive.
pub fn ensure_vec_extension_registered() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        // SAFETY: `sqlite3_vec_init` is the C entry point exported by the
        // sqlite-vec extension. We transmute its `extern "C" fn()` signature
        // to the SQLite extension entrypoint signature
        // (`unsafe extern "C" fn(*mut sqlite3, *mut *mut c_char, *const sqlite3_api_routines) -> i32`)
        // because sqlite-vec exposes the symbol without the extension
        // metadata wrapper. This matches the documented usage pattern in
        // the upstream `sqlite-vec` crate (see its `tests::test_rusqlite_auto_extension`).
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut std::os::raw::c_char,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::os::raw::c_int,
            >(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

/// Open (or create) the sqlite-vec backed database at `path` with the DEFAULT
/// feature-hash width ([`EMBED_DIM`] = 384).
///
/// Thin wrapper over [`open_db_with`]: keeps the historic single-arg signature
/// for every default call-site (and every test) byte-identical — the feature-hash
/// `chunks_vec(embedding float[384])` DDL and its vectors do not change. Paths
/// that index/query with a non-default embedder MUST call [`open_db_with`] with
/// `active_embedder(EMBED_DIM).dim()` so the DDL width matches the embedder that
/// produces the vectors (see [`open_db_with`] for the ordering rationale).
pub fn open_db(path: &Path) -> Result<Connection> {
    open_db_with(path, EMBED_DIM)
}

/// Open (or create) the sqlite-vec backed database at `path`, ensuring the
/// extension is registered and the schema (chunks + `chunks_vec`) exists, with
/// the `chunks_vec` vector width set to `dim`.
///
/// ## Why the dim is a CALLER argument (ordering)
///
/// `open_db` creates the `chunks_vec` DDL — a `float[N]` width FIXED at table
/// creation — but the active embedder (which decides `N` via [`Embedder::dim`])
/// is resolved LATER, inside the index/query path. So the only way to let a
/// non-default embedder (e.g. 768/256-wide) build its own index is for the
/// embedder-aware call-sites to resolve the active embedder FIRST and pass its
/// `dim()` here, BEFORE opening. The default path keeps passing [`EMBED_DIM`] via
/// [`open_db`], so the feature-hash index stays exactly `float[384]`.
///
/// The DDL width and the embedder that produced the vectors are then both pinned:
/// [`crate::schema::write_embedder_meta`] stamps `(id, dim)` on first write and
/// [`crate::schema::verify_embedder_meta`] refuses a later open with a different
/// dim/id, so a width that disagrees with the active embedder can never be queried
/// as garbage.
pub fn open_db_with(path: &Path, dim: usize) -> Result<Connection> {
    ensure_vec_extension_registered();
    let conn = Connection::open(path).context("open sqlite db")?;
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS chunks (
            id TEXT PRIMARY KEY,
            file_path TEXT NOT NULL,
            start_line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            body TEXT NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS chunks_vec USING vec0(
            embedding float[{dim}]
         );"
    ))
    .context("create schema (chunks + chunks_vec)")?;
    Ok(conn)
}

/// QUARANTINED: legacy INSERT OR REPLACE primitive — engine/server MUST use
/// insert_chunk_full. Retained only for preserved upstream tests.
///
/// Insert (or replace) a chunk and its embedding. Embedding is computed inline
/// from `chunk.body` via `feature_hash_embed`. The chunks_vec rowid is bound
/// to the chunks rowid so the JOIN in `knn_query` is constant-time.
#[doc(hidden)]
pub fn insert_chunk(conn: &Connection, chunk: &IndexedChunk) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO chunks (id, file_path, start_line, end_line, body) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            chunk.id,
            chunk.file_path,
            chunk.start_line,
            chunk.end_line,
            chunk.body
        ],
    )
    .context("insert chunk row")?;

    let embed = feature_hash_embed(&chunk.body, EMBED_DIM);
    let bytes: Vec<u8> = embed.iter().flat_map(|f| f.to_le_bytes()).collect();
    conn.execute(
        "INSERT OR REPLACE INTO chunks_vec (rowid, embedding) \
         VALUES ((SELECT rowid FROM chunks WHERE id = ?1), ?2)",
        params![chunk.id, bytes],
    )
    .context("insert chunk embedding")?;
    Ok(())
}

/// A symbol to attach to a chunk via [`insert_chunk_full`]. `kind` is the chunk
/// kind tag (e.g. `"function"`/`"method"`); `signature` is the rendered
/// single-line signature text.
#[derive(Debug, Clone)]
pub struct SymbolData {
    pub name: String,
    pub signature: String,
    pub kind: String,
    pub line: i64,
}

/// Sole write path for a chunk. Inserts the chunk row (with `kind`), its
/// embedding into `chunks_vec`, its tokenized body into `chunks_fts`, and — when
/// present — its symbol row.
///
/// Uses plain `INSERT` (NOT `INSERT OR REPLACE`): the caller is responsible for
/// deleting any prior rows for this id first (incremental delete lands in a
/// later step). This keeps the FTS/vec rowids consistent with the chunks rowid.
pub fn insert_chunk_full(
    conn: &Connection,
    chunk: &IndexedChunk,
    kind: &str,
    symbol: Option<&SymbolData>,
) -> Result<()> {
    // The DEFAULT write path uses the feature-hash embedder, producing vectors
    // byte-identical to the feature-hash-only engine (rrf_proof / vector tests stay
    // green). The pluggable variant is `insert_chunk_full_with`.
    let embedder = FeatureHashEmbedder::new(EMBED_DIM);
    insert_chunk_full_with(conn, chunk, kind, symbol, &embedder)
}

/// Embedder-aware write path. Identical to [`insert_chunk_full`]
/// but takes the active [`Embedder`] instead of hardcoding feature-hashing.
///
/// The embedder's `dim()` MUST match the `chunks_vec` DDL width ([`EMBED_DIM`]);
/// callers stamp the index's `meta(id, dim)` once so a mismatched embedder is
/// refused at open/query time (see [`crate::schema::verify_embedder_meta`]).
/// Passing [`FeatureHashEmbedder::new`]`(EMBED_DIM)` reproduces the legacy bytes
/// exactly.
pub fn insert_chunk_full_with(
    conn: &Connection,
    chunk: &IndexedChunk,
    kind: &str,
    symbol: Option<&SymbolData>,
    embedder: &dyn Embedder,
) -> Result<()> {
    conn.execute(
        "INSERT INTO chunks (id, file_path, start_line, end_line, body, kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            chunk.id,
            chunk.file_path,
            chunk.start_line,
            chunk.end_line,
            chunk.body,
            kind
        ],
    )
    .context("insert chunk row")?;

    // Cap the EMBEDDED + FTS-indexed text to MAX_CHUNK_BYTES. An oversized Symbol
    // chunk (a very large function — which we never split, since that would break
    // the <=1-symbol-per-chunk PK invariant) would otherwise produce a diluted
    // feature-hash embedding and a huge FTS document. The stored chunks.body
    // (inserted above) and the symbol row are kept whole. Module/Window chunks are
    // already bounded by the chunker, so this is a no-op for them.
    let indexed = truncate_on_char_boundary(&chunk.body, MAX_CHUNK_BYTES);

    let embed = embedder.embed_document(indexed);
    let bytes: Vec<u8> = embed.iter().flat_map(|f| f.to_le_bytes()).collect();
    conn.execute(
        "INSERT INTO chunks_vec (rowid, embedding) \
         VALUES ((SELECT rowid FROM chunks WHERE id = ?1), ?2)",
        params![chunk.id, bytes],
    )
    .context("insert chunk embedding")?;

    let body_tokens = code_tokens(indexed).join(" ");
    conn.execute(
        "INSERT INTO chunks_fts (rowid, body_tokens) \
         VALUES ((SELECT rowid FROM chunks WHERE id = ?1), ?2)",
        params![chunk.id, body_tokens],
    )
    .context("insert chunk fts row")?;

    if let Some(sym) = symbol {
        conn.execute(
            "INSERT INTO symbols (chunk_id, name, signature, kind, line) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chunk.id, sym.name, sym.signature, sym.kind, sym.line],
        )
        .context("insert symbol row")?;
    }

    Ok(())
}

/// Write a file's structural records (imports + exports) into `file_imports`
/// and `file_exports`. Each import/export becomes one row; `source`/`kind`/
/// `line` and `detail`/`line` are rendered from the parser statements.
pub fn write_file_structural(
    conn: &Connection,
    file_path: &str,
    imports: &[ImportStatement],
    exports: &[ExportStatement],
) -> Result<()> {
    for imp in imports {
        conn.execute(
            "INSERT INTO file_imports (file_path, source, kind, line) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                file_path,
                imp.source,
                import_kind_tag(&imp.import_kind),
                imp.line as i64
            ],
        )
        .context("insert file import row")?;
    }
    for exp in exports {
        conn.execute(
            "INSERT INTO file_exports (file_path, detail, line) \
             VALUES (?1, ?2, ?3)",
            params![file_path, export_detail(&exp.export_kind), exp.line as i64],
        )
        .context("insert file export row")?;
    }
    Ok(())
}

/// Stable tag for an import's kind, used as the `kind` column in `file_imports`.
fn import_kind_tag(kind: &ImportKind) -> &'static str {
    match kind {
        ImportKind::Named(_) => "named",
        ImportKind::Default(_) => "default",
        ImportKind::Namespace(_) => "namespace",
        ImportKind::SideEffect => "side_effect",
        ImportKind::Require(_) => "require",
    }
}

/// Render an export's `detail` column: a human-readable summary of what is
/// exported (names, default target, or re-export source).
fn export_detail(kind: &ExportKind) -> String {
    match kind {
        ExportKind::Named(items) => items.join(", "),
        ExportKind::Default(name) => format!("default {name}"),
        ExportKind::ReExport { items, source } => {
            format!("{} from {source}", items.join(", "))
        }
        ExportKind::ReExportAll(source) => format!("* from {source}"),
    }
}

/// K-nearest-neighbor search. Embeds `query` with the same feature-hashing
/// pipeline and asks vec0 for the closest `k` chunks. Returns `KnnHit`s in
/// ascending distance order.
pub fn knn_query(conn: &Connection, query: &str, k: usize) -> Result<Vec<KnnHit>> {
    // DEFAULT query path: feature-hash embedder → byte-identical query vectors.
    let embedder = FeatureHashEmbedder::new(EMBED_DIM);
    knn_query_with(conn, query, k, &embedder)
}

/// Embedder-aware KNN. Identical to [`knn_query`] but embeds
/// the query with the supplied active [`Embedder`] instead of hardcoding
/// feature-hashing. The embedder MUST be the SAME backend the index was built
/// with (enforced by [`crate::schema::verify_embedder_meta`] at the call site).
pub fn knn_query_with(
    conn: &Connection,
    query: &str,
    k: usize,
    embedder: &dyn Embedder,
) -> Result<Vec<KnnHit>> {
    let embed = embedder.embed_query(query);
    let bytes: Vec<u8> = embed.iter().flat_map(|f| f.to_le_bytes()).collect();
    let mut stmt = conn
        .prepare(
            "SELECT chunks.id, chunks_vec.distance \
             FROM chunks_vec \
             INNER JOIN chunks ON chunks.rowid = chunks_vec.rowid \
             WHERE embedding MATCH ?1 AND k = ?2 \
             ORDER BY distance",
        )
        .context("prepare knn statement")?;
    let rows = stmt
        .query_map(params![bytes, k as i64], |row| {
            Ok(KnnHit {
                chunk_id: row.get(0)?,
                distance: row.get(1)?,
            })
        })
        .context("execute knn query")?;
    let mut hits = Vec::new();
    for r in rows {
        hits.push(r.context("read knn row")?);
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::migrate;
    use tempfile::tempdir;

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn insert_chunk_full_round_trip() {
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("idx.sqlite")).unwrap();
        migrate(&conn).unwrap();

        let chunk = IndexedChunk {
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

        insert_chunk_full(&conn, &chunk, "function", Some(&sym)).unwrap();

        assert_eq!(count(&conn, "SELECT COUNT(*) FROM chunks"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM chunks_vec"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM chunks_fts"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM symbols"), 1);

        // kind is stored on the chunk row.
        let kind: String = conn
            .query_row(
                "SELECT kind FROM chunks WHERE id = ?1",
                params![chunk.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "function");

        // A code_tokens term ("parse", split from parseString) matches via FTS
        // and resolves back to the chunk's rowid.
        let want_rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM chunks WHERE id = ?1",
                params![chunk.id],
                |row| row.get(0),
            )
            .unwrap();
        let got_rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH ?1",
                params!["parse"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(got_rowid, want_rowid);
    }

    #[test]
    fn chunk_caps_oversized_symbol() {
        // The byte-cap helper is a no-op below the limit and truncates on a UTF-8
        // char boundary above it (never splits a multibyte char).
        assert_eq!(truncate_on_char_boundary("short", MAX_CHUNK_BYTES), "short");
        let multibyte = "é".repeat(MAX_CHUNK_BYTES); // 2 bytes each => over the cap
        let capped = truncate_on_char_boundary(&multibyte, MAX_CHUNK_BYTES);
        assert!(capped.len() <= MAX_CHUNK_BYTES);
        assert!(multibyte.starts_with(capped)); // a valid UTF-8 prefix

        // An oversized Symbol chunk (body well over MAX_CHUNK_BYTES) is stored
        // WHOLE, keeps its symbol row + signature, and still indexes — the cap
        // only bounds the embedded/FTS input, never the stored body or symbol.
        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("idx.sqlite")).unwrap();
        migrate(&conn).unwrap();

        let huge_body = format!(
            "fn giant() {{\n{}}}",
            "    let _x = compute_value();\n".repeat(MAX_CHUNK_BYTES / 10)
        );
        assert!(huge_body.len() > MAX_CHUNK_BYTES);
        let chunk = IndexedChunk {
            id: "src/big.rs:1-9999".to_string(),
            file_path: "src/big.rs".to_string(),
            start_line: 1,
            end_line: 9999,
            body: huge_body.clone(),
        };
        let sym = SymbolData {
            name: "giant".to_string(),
            signature: "fn giant()".to_string(),
            kind: "function".to_string(),
            line: 1,
        };
        insert_chunk_full(&conn, &chunk, "function", Some(&sym)).unwrap();

        // Exactly one symbol row, with the signature preserved.
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM symbols"), 1);
        let stored_sig: String = conn
            .query_row(
                "SELECT signature FROM symbols WHERE chunk_id = ?1",
                params![chunk.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored_sig, "fn giant()");

        // The FULL body is stored (the cap never truncates chunks.body).
        let stored_len: i64 = conn
            .query_row(
                "SELECT length(body) FROM chunks WHERE id = ?1",
                params![chunk.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored_len as usize, huge_body.len());

        // Still searchable: the capped FTS input keeps the early tokens.
        assert!(
            count(
                &conn,
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'compute'"
            ) >= 1
        );
    }

    /// Story 2 AC1: `open_db_with(path, dim)` parametrizes the `chunks_vec` DDL
    /// width — a non-default dim produces `float[<dim>]`, while `open_db` (and
    /// `open_db_with(_, EMBED_DIM)`) stay at the feature-hash default `float[384]`.
    #[test]
    fn open_db_with_parametrizes_vec_width() {
        // The DDL text recorded in sqlite_master carries the declared vec width.
        let vec_ddl = |conn: &Connection| -> String {
            conn.query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'chunks_vec'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        };

        // Default open is byte-identical to the historic float[384] DDL.
        let d384 = tempdir().unwrap();
        let conn384 = open_db(&d384.path().join("idx.sqlite")).unwrap();
        assert_eq!(EMBED_DIM, 384);
        assert!(
            vec_ddl(&conn384).contains("float[384]"),
            "default open_db must keep float[384], got: {}",
            vec_ddl(&conn384)
        );

        // A non-default width is honored by open_db_with.
        let d768 = tempdir().unwrap();
        let conn768 = open_db_with(&d768.path().join("idx.sqlite"), 768).unwrap();
        let ddl768 = vec_ddl(&conn768);
        assert!(
            ddl768.contains("float[768]") && !ddl768.contains("float[384]"),
            "open_db_with(_, 768) must declare float[768], got: {ddl768}"
        );

        // Functional check: a 768-wide vector inserts + KNN-queries against the
        // 768 index; the same width is what makes the DDL/embedder agree.
        migrate(&conn768).unwrap();
        let e768 = crate::embedder::FeatureHashEmbedder::new(768);
        let chunk = IndexedChunk {
            id: "src/x.rs:1-1".to_string(),
            file_path: "src/x.rs".to_string(),
            start_line: 1,
            end_line: 1,
            body: "fn wide() {}".to_string(),
        };
        insert_chunk_full_with(&conn768, &chunk, "function", None, &e768).unwrap();
        assert_eq!(count(&conn768, "SELECT COUNT(*) FROM chunks_vec"), 1);
        let hits = knn_query_with(&conn768, "fn wide() {}", 1, &e768).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk_id, "src/x.rs:1-1");
    }

    #[test]
    fn write_file_structural_inserts_rows() {
        use crate::parser::{ExportKind, ExportStatement, ImportKind, ImportStatement};

        let dir = tempdir().unwrap();
        let conn = open_db(&dir.path().join("s.sqlite")).unwrap();
        migrate(&conn).unwrap();

        let imports = vec![
            ImportStatement::new("react", ImportKind::Default("React".into())).with_line(1),
            ImportStatement::new("./utils", ImportKind::Named(vec!["a".into(), "b".into()]))
                .with_line(2),
        ];
        let exports =
            vec![ExportStatement::new(ExportKind::Named(vec!["foo".into()])).with_line(5)];

        write_file_structural(&conn, "src/app.ts", &imports, &exports).unwrap();

        assert_eq!(count(&conn, "SELECT COUNT(*) FROM file_imports"), 2);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM file_exports"), 1);
    }
}
