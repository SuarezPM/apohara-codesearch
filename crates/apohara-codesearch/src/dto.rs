// SPDX-License-Identifier: MIT OR Apache-2.0

//! Wire DTOs returned by the `search_code` tool.
//!
//! These are the JSON-serializable, schema-bearing projection of the engine's
//! [`apohara_indexer::HydratedHit`]. The engine type carries an internal
//! `chunk_id` we deliberately drop from the wire shape, and we rename
//! `file_path` to `file` to match the documented MCP tool contract.

use apohara_indexer::{ExportRow, HydratedHit, ImportRow, ReindexReport};
use schemars::JsonSchema;
use serde::Serialize;

/// Object wrapper around the search hits.
///
/// rmcp 1.7 enforces the MCP spec rule that a tool's `outputSchema` root must
/// be an `object`; a bare array (`Vec<SearchHit>`) is rejected at startup. The
/// `hits` field gives the result an object root.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResults {
    /// Top-k hits ordered by descending fused score.
    pub hits: Vec<SearchHit>,
}

/// One hydrated search result with its fused relevance score.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchHit {
    /// Repo-relative path of the file the chunk came from.
    pub file: String,
    /// 1-based first line of the chunk (inclusive).
    pub start_line: i64,
    /// 1-based last line of the chunk (inclusive).
    pub end_line: i64,
    /// Chunk kind (e.g. `function`, `module`, `window`).
    pub kind: String,
    /// Symbol signature when the chunk is a named symbol, else `None`.
    pub signature: Option<String>,
    /// The chunk source text (capped by the engine's snippet limit).
    pub snippet: String,
    /// Reciprocal-rank-fusion score from the BM25 + vector fuse.
    pub score: f64,
    /// Import statements discovered in the chunk's file.
    pub imports: Vec<ImportDto>,
    /// Export statements discovered in the chunk's file.
    pub exports: Vec<ExportDto>,
}

/// Import row projection of [`apohara_indexer::ImportRow`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ImportDto {
    pub source: String,
    pub kind: String,
    pub line: i64,
}

/// Export row projection of [`apohara_indexer::ExportRow`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExportDto {
    pub detail: String,
    pub line: i64,
}

impl From<&ImportRow> for ImportDto {
    fn from(r: &ImportRow) -> Self {
        Self {
            source: r.source.clone(),
            kind: r.kind.clone(),
            line: r.line,
        }
    }
}

impl From<&ExportRow> for ExportDto {
    fn from(r: &ExportRow) -> Self {
        Self {
            detail: r.detail.clone(),
            line: r.line,
        }
    }
}

/// Wire projection of [`apohara_indexer::ReindexReport`].
///
/// The engine type derives only `Serialize`; this DTO adds the `JsonSchema`
/// the `Json<T>` tool-output wrapper requires, keeping the indexer crate free
/// of a schemars dependency.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ReindexReportDto {
    /// Number of files (re)indexed this run.
    pub files_indexed: usize,
    /// Total chunks written across all (re)indexed files this run.
    pub chunks: usize,
    /// Wall-clock duration of the reindex in milliseconds.
    pub duration_ms: u64,
    /// `false` for a full (force) reindex, `true` for an incremental one.
    pub incremental: bool,
}

impl From<ReindexReport> for ReindexReportDto {
    fn from(r: ReindexReport) -> Self {
        Self {
            files_indexed: r.files_indexed,
            chunks: r.chunks,
            // u128 -> u64: indexing durations never approach the u64 ms ceiling.
            duration_ms: r.duration_ms as u64,
            incremental: r.incremental,
        }
    }
}

impl SearchHit {
    /// Build a wire hit from an engine [`HydratedHit`] and its fused score.
    pub fn from_hydrated(hit: HydratedHit, score: f64) -> Self {
        Self {
            file: hit.file_path,
            start_line: hit.start_line,
            end_line: hit.end_line,
            kind: hit.kind,
            signature: hit.signature,
            snippet: hit.snippet,
            score,
            imports: hit.imports.iter().map(ImportDto::from).collect(),
            exports: hit.exports.iter().map(ExportDto::from).collect(),
        }
    }
}
