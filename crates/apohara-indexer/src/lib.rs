// SPDX-License-Identifier: MIT OR Apache-2.0

//! Apohara code indexer (soft-fork): sqlite-vec storage + blake3
//! feature-hashing embeddings + tree-sitter parsing.
//!
//! This crate is a LIB-ONLY soft-fork of `apohara-indexer` from
//! SuarezPM/Apohara-Catalyst. The storage, parser, and embeddings modules are
//! preserved verbatim; the binary entry point (`main.rs`) is intentionally
//! dropped.
//!
//! The new engine modules land in subsequent steps. Step 1 adds `walker`
//! (gitignore-aware traversal) and `chunker` (symbol/module/window chunking);
//! `schema`/`search`/`incremental` follow later.

pub mod chunker;
pub mod embedder;
pub mod embeddings;
pub mod incremental;
pub mod parser;
pub mod schema;
pub mod search;
pub mod storage;
pub mod tokens;
pub mod walker;

pub use storage::{
    ensure_vec_extension_registered, insert_chunk, insert_chunk_full, insert_chunk_full_with,
    knn_query, knn_query_with, open_db, write_file_structural, IndexedChunk, KnnHit, SymbolData,
    EMBED_DIM,
};

pub use embedder::{
    active_embedder, resolve_embedder_choice, Embedder, EmbedderChoice, FeatureHashEmbedder,
    EMBED_MODEL_ENV, FEATURE_HASH_ID,
};

pub use schema::{
    migrate, read_embedder_meta, verify_embedder_meta, write_embedder_meta, META_EMBEDDER_DIM,
    META_EMBEDDER_ID,
};

pub use search::{
    apply_structural_boost, bm25_query, classify_query_weights, dedup_content, dedup_overlapping,
    hydrate, load_embeddings, mmr_rerank, resolve_weights, rrf_fuse, rrf_fuse_weighted,
    vector_query, vector_query_with, ExportRow, HydratedHit, ImportRow, MMR_LAMBDA, RRF_K,
    STRUCTURAL_BOOST,
};

pub use tokens::code_tokens;

pub use embeddings::feature_hash_embed;

pub use parser::{
    detect_language, parse_file, parse_imports_exports, parse_source, parse_source_imports_exports,
    parse_source_spans, ExportStatement, FunctionSignature, ImportStatement, Language, SymbolKind,
};

pub use walker::{walk_repo, WalkedFile};

pub use chunker::{chunk_file, chunk_id, ChunkKind, ChunkSpec};

pub use incremental::{index_repo, reindex, ReindexReport};

/// Bundled SQLite version string (e.g. `"3.46.0"`).
///
/// Exposed so the `apohara-codesearch` binary can report the version compiled
/// into rusqlite's `bundled` feature without taking a direct rusqlite dep.
pub fn sqlite_version() -> &'static str {
    rusqlite::version()
}
