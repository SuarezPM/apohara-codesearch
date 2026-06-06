// SPDX-License-Identifier: MIT OR Apache-2.0

//! The stdio MCP server: exactly two tools (`search_code`, `reindex`) wired to
//! the reused `apohara-indexer` engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use apohara_indexer::{
    active_embedder, apply_structural_boost, bm25_query, dedup_content, dedup_overlapping, hydrate,
    index_repo, load_embeddings, migrate, mmr_rerank, open_db, reindex, resolve_weights,
    rrf_fuse_weighted, vector_query_with, verify_embedder_meta, EMBED_DIM, MMR_LAMBDA, RRF_K,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::dto::{ReindexReportDto, SearchHit, SearchResults};

/// Default number of results when the caller omits `k`.
const DEFAULT_K: usize = 10;

/// Sub-directory under the repo root holding the index database and build lock.
const INDEX_DIR: &str = ".apohara-codesearch";

/// Input for the `search_code` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchCodeParams {
    /// Absolute path to the repository root to search.
    pub path: String,
    /// Natural-language or code query.
    pub query: String,
    /// Max results to return (defaults to 10).
    pub k: Option<usize>,
    /// RRF constant `k_rrf` (defaults to 60). Larger flattens top-rank
    /// contributions; smaller sharpens reordering.
    pub rrf_k: Option<usize>,
    /// Weight on the BM25 (lexical) list's RRF contribution. When set, this
    /// explicit value always wins, even with `adaptive=true`. Defaults to the
    /// adaptive heuristic if `adaptive=true`, else 1.0.
    pub bm25_weight: Option<f64>,
    /// Weight on the vector list's RRF contribution. When set, this explicit
    /// value always wins, even with `adaptive=true`. Defaults to the adaptive
    /// heuristic if `adaptive=true`, else 1.0.
    pub vector_weight: Option<f64>,
    /// Opt-in adaptive fusion: pick the fusion weights from the SHAPE of the
    /// query (identifier/snake_case/camelCase → BM25-heavy; multi-word NL →
    /// vector-heavy). Only fills weights the caller left unset; an explicit
    /// `bm25_weight`/`vector_weight` always overrides it. Defaults to false.
    /// LIMITATION: lexical-only, no corpus signal — and the vector arm is
    /// near-noise with the default feature-hash embedder (see BENCHMARK.md).
    pub adaptive: Option<bool>,
    /// Apply Maximal Marginal Relevance diversity re-ranking after fusion +
    /// dedup (defaults to false → plain fused order).
    pub diversify: Option<bool>,
    /// Apply the structural import-match boost to candidates whose file imports
    /// a query-matching source (defaults to false → no boost).
    pub boost_imports: Option<bool>,
}

/// Input for the `reindex` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReindexParams {
    /// Absolute path to the repository root to (re)index.
    pub path: String,
    /// Force a full wipe-and-rebuild instead of an incremental diff.
    pub force: Option<bool>,
}

/// The MCP server handler holding the macro-generated tool router.
#[derive(Clone)]
pub struct CodeSearchServer {
    // Read by the #[tool_handler]/#[tool_router] macro expansion; the dead_code
    // lint cannot see that cross-macro read, so silence the false positive.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl CodeSearchServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for CodeSearchServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve the index database path `<root>/.apohara-codesearch/index.db`,
/// creating the parent directory if needed (rusqlite's `Connection::open`
/// does not create missing parent dirs).
fn db_path(root: &Path) -> Result<PathBuf, ErrorData> {
    let dir = root.join(INDEX_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| internal(format!("create index dir {}: {e}", dir.display())))?;
    Ok(dir.join("index.db"))
}

/// Process-level map of per-repo build locks, keyed by canonical repo path.
/// Guarantees two racing `search_code` calls on the same repo do not both run
/// the lazy first-index concurrently (which would double-build the database).
fn build_locks() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Fetch (or create) the build lock for `root`.
fn build_lock_for(root: &Path) -> Arc<Mutex<()>> {
    // Canonicalize when possible so different spellings of the same repo share
    // one lock; fall back to the raw path if the dir does not yet exist.
    let key = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut map = build_locks().lock().expect("build-locks mutex poisoned");
    map.entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Lazily build the index on first use, guarded by the per-repo build lock.
/// No-op when the database file already exists.
fn ensure_indexed(root: &Path, db: &Path) -> Result<(), ErrorData> {
    if db.exists() {
        return Ok(());
    }
    let lock = build_lock_for(root);
    let _guard = lock.lock().expect("build lock poisoned");
    // Re-check under the lock: a racing caller may have built it while we waited.
    if db.exists() {
        return Ok(());
    }
    let conn = open_db(db).map_err(|e| internal(format!("open_db: {e}")))?;
    migrate(&conn).map_err(|e| internal(format!("migrate: {e}")))?;
    // Lazy first-index MUST go through index_repo (insert_chunk_full), never the
    // quarantined insert_chunk primitive.
    index_repo(&conn, root).map_err(|e| internal(format!("index_repo: {e}")))?;
    Ok(())
}

#[tool_router]
impl CodeSearchServer {
    /// Hybrid (BM25 + vector, RRF-fused) code search over a repository. Lazily
    /// builds the index on first use, then returns the top-k hydrated hits.
    #[tool(
        name = "search_code",
        description = "Search a code repository with hybrid BM25 + vector retrieval (RRF-fused). Lazily indexes the repo on first call. Returns the top-k hydrated chunks with file, line range, kind, signature, snippet, score, and the file's imports/exports. Set adaptive=true (opt-in, default off) to pick fusion weights from the query shape (identifier/snake_case/camelCase up-weight BM25; multi-word NL up-weights vector); explicit bm25_weight/vector_weight always override it. LIMITATION: adaptive is lexical-only (no corpus signal) and the vector arm is near-noise with the default feature-hash embedder."
    )]
    async fn search_code(
        &self,
        Parameters(SearchCodeParams {
            path,
            query,
            k,
            rrf_k,
            bm25_weight,
            vector_weight,
            adaptive,
            diversify,
            boost_imports,
        }): Parameters<SearchCodeParams>,
    ) -> Result<Json<SearchResults>, ErrorData> {
        let root = PathBuf::from(&path);
        let db = db_path(&root)?;
        ensure_indexed(&root, &db)?;

        let k = k.unwrap_or(DEFAULT_K);
        let rrf_k = rrf_k.unwrap_or(RRF_K);
        // Defer the weight unwrap until AFTER the adaptive branch: resolve_weights
        // keeps the caller's Option<f64> intent alive so an explicit weight wins
        // over the heuristic (precedence: explicit > adaptive > 1.0/1.0 default).
        let (bm25_weight, vector_weight) = resolve_weights(
            bm25_weight,
            vector_weight,
            adaptive.unwrap_or(false),
            &query,
        );
        let diversify = diversify.unwrap_or(false);
        let boost_imports = boost_imports.unwrap_or(false);
        let conn = open_db(&db).map_err(|e| internal(format!("open_db: {e}")))?;
        migrate(&conn).map_err(|e| internal(format!("migrate: {e}")))?;

        // Resolve the active embedder (feature-hash by default; a local model
        // only with the `gguf-embed` feature + a present model path). Refuse to
        // query an index that was built with a different embedder (id/dim) — a
        // clear error beats silently mis-ranking against incompatible vectors.
        let embedder = active_embedder(EMBED_DIM);
        verify_embedder_meta(&conn, embedder.id(), embedder.dim())
            .map_err(|e| internal(format!("verify_embedder_meta: {e}")))?;

        let bm25 =
            bm25_query(&conn, &query, k).map_err(|e| internal(format!("bm25_query: {e}")))?;
        let vector = vector_query_with(&conn, &query, k, embedder.as_ref())
            .map_err(|e| internal(format!("vector_query: {e}")))?;
        let mut fused = rrf_fuse_weighted(&bm25, &vector, rrf_k, bm25_weight, vector_weight);

        // Optional structural import boost over the fused list
        // (config-gated, default off → ordering unchanged). Runs before dedup so
        // a boosted chunk competes for its survivor slot at its boosted rank.
        if boost_imports {
            apply_structural_boost(&conn, &query, &mut fused)
                .map_err(|e| internal(format!("apply_structural_boost: {e}")))?;
        }

        // 1c Stage A — same-file overlapping-range dedup over the fused ids
        // (no DB read). Runs BEFORE take-k so the trim is not padded with
        // near-duplicates of higher-ranked survivors.
        dedup_overlapping(&mut fused);

        // Optional MMR diversity re-rank after fusion + dedup,
        // before take-k (config-gated, default off → plain fused order). Reuses
        // the persisted feature-hash vectors as the similarity signal.
        if diversify {
            let ids: Vec<String> = fused.iter().map(|(id, _)| id.clone()).collect();
            let embeddings = load_embeddings(&conn, &ids)
                .map_err(|e| internal(format!("load_embeddings: {e}")))?;
            fused = mmr_rerank(&fused, &embeddings, MMR_LAMBDA);
        }

        // Hydrate the surviving top-k, carrying each fused score alongside its
        // hit, then 1c Stage B — cross-file content dedup over the
        // already-materialized hits (no query beyond hydrate).
        let mut hydrated = Vec::with_capacity(k.min(fused.len()));
        for (chunk_id, score) in fused.into_iter().take(k) {
            if let Some(hit) =
                hydrate(&conn, &chunk_id).map_err(|e| internal(format!("hydrate: {e}")))?
            {
                hydrated.push((hit, score));
            }
        }
        dedup_content(&mut hydrated);

        let hits = hydrated
            .into_iter()
            .map(|(hit, score)| SearchHit::from_hydrated(hit, score))
            .collect();
        Ok(Json(SearchResults { hits }))
    }

    /// (Re)index a repository. Incremental by default; pass `force: true` for a
    /// full wipe-and-rebuild. Returns the reindex report.
    #[tool(
        name = "reindex",
        description = "(Re)index a code repository. Incremental by default (content-hash diff); pass force=true for a full wipe-and-rebuild. Returns counts of files/chunks indexed and the duration."
    )]
    async fn reindex(
        &self,
        Parameters(ReindexParams { path, force }): Parameters<ReindexParams>,
    ) -> Result<Json<ReindexReportDto>, ErrorData> {
        let root = PathBuf::from(&path);
        let db = db_path(&root)?;
        let conn = open_db(&db).map_err(|e| internal(format!("open_db: {e}")))?;
        migrate(&conn).map_err(|e| internal(format!("migrate: {e}")))?;
        let report = reindex(&conn, &root, force.unwrap_or(false))
            .map_err(|e| internal(format!("reindex: {e}")))?;
        Ok(Json(report.into()))
    }
}

#[tool_handler]
impl ServerHandler for CodeSearchServer {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo is #[non_exhaustive]: start from Default and set fields.
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "apohara-codesearch MCP server. Tools: search_code (hybrid BM25+vector search, \
             lazy first-index) and reindex (incremental/full rebuild)."
                .to_string(),
        );
        info
    }
}

/// Build an internal-error `ErrorData` from a message.
fn internal(msg: String) -> ErrorData {
    ErrorData::internal_error(msg, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These guard the exact precedence the search_code handler wires through
    // resolve_weights (server.rs ~line 162). They exercise the same function the
    // handler calls, so the async DB path does not need to be stood up to test
    // the weight-resolution contract.

    /// AC2: an explicit caller weight wins over adaptive, even on an identifier
    /// query the heuristic would otherwise bias BM25-heavy.
    #[test]
    fn search_code_explicit_weights_override_adaptive() {
        let (bm25, vector) = resolve_weights(Some(0.25), Some(0.75), true, "parse_config");
        assert_eq!((bm25, vector), (0.25, 0.75));
    }

    /// AC3: adaptive absent/false with no explicit weights resolves to the legacy
    /// 1.0/1.0 default — the same weights the pre-adaptive handler passed in.
    #[test]
    fn search_code_default_is_legacy_neutral() {
        let (bm25, vector) = resolve_weights(None, None, false, "parse_config");
        assert_eq!((bm25, vector), (1.0, 1.0));
    }

    /// Adaptive on with no explicit weights drives the heuristic; an identifier
    /// query is BM25-heavy.
    #[test]
    fn search_code_adaptive_identifier_is_bm25_heavy() {
        let (bm25, vector) = resolve_weights(None, None, true, "parse_config");
        assert!(bm25 > vector);
    }
}
