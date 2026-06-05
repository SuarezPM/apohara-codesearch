// SPDX-License-Identifier: MIT OR Apache-2.0

//! Pluggable embedding behind the [`Embedder`] trait.
//!
//! ## The invariant this module protects
//!
//! The default build ships no model. The DEFAULT build (no Cargo features)
//! compiles ONLY [`FeatureHashEmbedder`], which wraps
//! [`crate::embeddings::feature_hash_embed`] BYTE-FOR-BYTE: the same buckets, the
//! same signs, the same L2 normalization. So with no feature flag set, every
//! vector written and every query embedded is identical to the feature-hash-only
//! engine, and `rrf_proof.rs` plus every vector/dedup test stays green.
//!
//! A SECOND backend, [`GgufEmbedder`], is compiled ONLY behind the
//! `gguf-embed` feature. It loads a USER-SUPPLIED local model file (never
//! vendored, never downloaded by us — see the module-level note on the loader).
//! With the feature OFF, none of its dependencies are pulled in and the
//! model-weight scan stays 0.
//!
//! ## Active-embedder selection (runtime)
//!
//! [`active_embedder`] resolves which backend to use:
//!   - If the `gguf-embed` feature is OFF, it is ALWAYS [`FeatureHashEmbedder`].
//!   - If the feature is ON and `APOHARA_EMBED_MODEL` points at an existing
//!     file, it tries [`GgufEmbedder`]; on any load error OR an absent path it
//!     falls back to [`FeatureHashEmbedder`] with a STDERR warning. It NEVER
//!     fetches over the network and NEVER panics.
//!
//! The fallback DECISION (`resolve_embedder_choice`) is pure and unit-tested
//! even with the feature OFF, so the no-model behavior is provable without a model.

/// The active-embedder environment variable: an absolute path to a user-supplied
/// local model file. Read only when the `gguf-embed` feature is compiled in.
/// Absent / empty → the default feature-hash embedder.
pub const EMBED_MODEL_ENV: &str = "APOHARA_EMBED_MODEL";

/// A pluggable text → vector embedder.
///
/// `id()` and `dim()` are persisted in the `meta` table so an index can refuse
/// to mix embeddings produced by different backends (see
/// [`crate::schema::read_embedder_meta`]). `embed` MUST be deterministic for a
/// given backend so re-indexing is stable.
pub trait Embedder: Send + Sync {
    /// Embed `text` into a `dim()`-length vector.
    fn embed(&self, text: &str) -> Vec<f32>;
    /// Stable backend identifier (e.g. `"feature-hash-v1"`). Recorded in `meta`.
    fn id(&self) -> &str;
    /// Output dimension. MUST match the `chunks_vec` DDL width for the index.
    fn dim(&self) -> usize;
}

/// Stable id for the default feature-hash backend, recorded in `meta.embedder_id`.
pub const FEATURE_HASH_ID: &str = "feature-hash-v1";

/// The default, always-compiled embedder: a thin wrapper over
/// [`crate::embeddings::feature_hash_embed`].
///
/// BYTE-IDENTICAL to calling `feature_hash_embed(text, dim)` directly — it adds
/// no transformation. This is what keeps `rrf_proof.rs` and all vector behavior
/// unchanged once the engine routes embedding through the [`Embedder`] trait.
#[derive(Debug, Clone)]
pub struct FeatureHashEmbedder {
    dim: usize,
}

impl FeatureHashEmbedder {
    /// Construct a feature-hash embedder producing `dim`-length vectors. Pass
    /// [`crate::storage::EMBED_DIM`] for the engine default (384).
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Embedder for FeatureHashEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        crate::embeddings::feature_hash_embed(text, self.dim)
    }
    fn id(&self) -> &str {
        FEATURE_HASH_ID
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

/// The decision made by [`resolve_embedder_choice`]: which backend to activate
/// and (when falling back) why. Pure data so the fallback logic is unit-testable
/// WITHOUT the `gguf-embed` feature or a real model file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedderChoice {
    /// Use the default feature-hash embedder (the no-model default).
    FeatureHash,
    /// Use the gguf/local-model embedder loaded from this path.
    Gguf { model_path: String },
    /// A model path was configured but unusable (absent, or the feature is off).
    /// The engine MUST fall back to feature-hash and emit `warning` on stderr.
    FallbackToFeatureHash { warning: String },
}

/// Pure fallback-decision logic (testable with the feature OFF).
///
/// - `configured_path = None`/empty → [`EmbedderChoice::FeatureHash`] (no warning:
///   no model was ever requested, this is the plain no-model default).
/// - `configured_path = Some(p)` but `path_exists(p) == false` →
///   [`EmbedderChoice::FallbackToFeatureHash`] with a warning (a model WAS
///   requested but is missing — never a network fetch, never a panic).
/// - `configured_path = Some(p)` and it exists → [`EmbedderChoice::Gguf`]. When
///   the `gguf-embed` feature is OFF the caller maps this to a fallback too,
///   because no loader is compiled — see [`active_embedder`].
pub fn resolve_embedder_choice(
    configured_path: Option<&str>,
    path_exists: impl Fn(&str) -> bool,
) -> EmbedderChoice {
    match configured_path {
        None => EmbedderChoice::FeatureHash,
        Some(p) if p.trim().is_empty() => EmbedderChoice::FeatureHash,
        Some(p) if path_exists(p) => EmbedderChoice::Gguf {
            model_path: p.to_string(),
        },
        Some(p) => EmbedderChoice::FallbackToFeatureHash {
            warning: format!(
                "apohara-codesearch: configured embedding model '{p}' not found; \
                 falling back to the built-in feature-hash embedder (no network fetch)"
            ),
        },
    }
}

/// Read the configured model path from [`EMBED_MODEL_ENV`]. Always `None` when
/// the `gguf-embed` feature is OFF, so the default build never even consults the
/// environment — the no-model default is structurally guaranteed.
fn configured_model_path() -> Option<String> {
    #[cfg(feature = "gguf-embed")]
    {
        std::env::var(EMBED_MODEL_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
    }
    #[cfg(not(feature = "gguf-embed"))]
    {
        None
    }
}

/// Resolve the active [`Embedder`] for this process.
///
/// Ships no model by default: with the `gguf-embed` feature OFF this is ALWAYS a
/// [`FeatureHashEmbedder`] of width `dim` and the environment is not consulted.
/// With the feature ON, a configured-and-present model path yields a
/// [`GgufEmbedder`]; an absent path (or any load failure) logs a stderr warning
/// and returns the feature-hash embedder. Never fetches over the network, never
/// panics.
pub fn active_embedder(dim: usize) -> Box<dyn Embedder> {
    let choice = resolve_embedder_choice(configured_model_path().as_deref(), |p| {
        std::path::Path::new(p).exists()
    });
    match choice {
        EmbedderChoice::FeatureHash => Box::new(FeatureHashEmbedder::new(dim)),
        EmbedderChoice::FallbackToFeatureHash { warning } => {
            eprintln!("{warning}");
            Box::new(FeatureHashEmbedder::new(dim))
        }
        EmbedderChoice::Gguf { model_path } => load_gguf_or_fallback(&model_path, dim),
    }
}

/// Load the gguf/local-model embedder, falling back to feature-hash on any error.
#[cfg(feature = "gguf-embed")]
fn load_gguf_or_fallback(model_path: &str, dim: usize) -> Box<dyn Embedder> {
    match GgufEmbedder::load(model_path, dim) {
        Ok(e) => Box::new(e),
        Err(err) => {
            eprintln!(
                "apohara-codesearch: failed to load embedding model '{model_path}': {err}; \
                 falling back to the built-in feature-hash embedder (no network fetch)"
            );
            Box::new(FeatureHashEmbedder::new(dim))
        }
    }
}

/// With the feature OFF this path is reached only if a future caller hand-builds
/// an `EmbedderChoice::Gguf`; it degrades to feature-hash with a warning so the
/// no-loader build can never attempt model inference.
#[cfg(not(feature = "gguf-embed"))]
fn load_gguf_or_fallback(model_path: &str, dim: usize) -> Box<dyn Embedder> {
    eprintln!(
        "apohara-codesearch: embedding model '{model_path}' requested but this binary \
         was built without the 'gguf-embed' feature; falling back to feature-hash"
    );
    Box::new(FeatureHashEmbedder::new(dim))
}

// ---------------------------------------------------------------------------
// gguf-embed backend (feature-gated). Compiled ONLY with `--features gguf-embed`.
// ---------------------------------------------------------------------------

/// Local-model embedder loaded from a USER-SUPPLIED file.
///
/// ## What is wired vs. scaffolded (honest status)
///
/// This struct OWNS the offline integration point: it loads the user-supplied
/// local model file from disk with `std::fs` ONLY — there is NO network code on
/// this path and no `hf-hub` dependency (web research 2026-06: candle and
/// `safetensors` both load directly from a local path; `hf-hub` is only the
/// optional downloader, which we do not use). The model file is ALWAYS supplied
/// by the user via [`EMBED_MODEL_ENV`]; we never vendor or download it.
///
/// The actual transformer forward-pass (candle BertModel / quantized GGUF) is an
/// INTEGRATION POINT, not vendored inference: completing it requires picking the
/// concrete architecture for the user's checkpoint and pulling `candle-core` /
/// `candle-transformers` / `tokenizers`. Because we cannot vendor a model to
/// test the real round-trip (that would itself break the no-model default), the inference
/// body is deliberately a single clearly-marked stub returning a zero vector of
/// the declared dim — never a panic. Everything AROUND it — the trait wiring, the
/// local-file load, the meta refuse-to-mix check, and the fallback-on-error — is
/// real and tested.
#[cfg(feature = "gguf-embed")]
#[derive(Debug)]
pub struct GgufEmbedder {
    id: String,
    dim: usize,
    #[allow(dead_code)] // Held for the inference integration point below.
    model_bytes: Vec<u8>,
}

#[cfg(feature = "gguf-embed")]
impl GgufEmbedder {
    /// Load a user-supplied local model file from `model_path`. Reads the file
    /// from disk with `std::fs` — NO network, NO `hf-hub`. Returns an error
    /// (never panics) when the file is unreadable, so the caller falls back.
    pub fn load(model_path: &str, dim: usize) -> anyhow::Result<Self> {
        use anyhow::Context;
        let model_bytes = std::fs::read(model_path)
            .with_context(|| format!("read local embedding model file '{model_path}'"))?;
        anyhow::ensure!(
            !model_bytes.is_empty(),
            "embedding model file '{model_path}' is empty"
        );
        // INTEGRATION POINT: parse the checkpoint (safetensors/GGUF) and build
        // the candle model here. Intentionally not vendored — see the struct doc.
        let id = format!("gguf:{}", file_stem(model_path));
        Ok(Self {
            id,
            dim,
            model_bytes,
        })
    }
}

#[cfg(feature = "gguf-embed")]
impl Embedder for GgufEmbedder {
    fn embed(&self, _text: &str) -> Vec<f32> {
        // INTEGRATION POINT: run the candle forward pass + mean-pooling here.
        // Until a concrete architecture is wired, return a zero vector of the
        // declared dim so callers never panic; `active_embedder` only reaches
        // this once a real model is present, and the meta check guards mixing.
        vec![0.0f32; self.dim]
    }
    fn id(&self) -> &str {
        &self.id
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

/// File stem (no directory, no extension) used to tag a gguf embedder's id.
#[cfg(feature = "gguf-embed")]
fn file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::feature_hash_embed;
    use crate::storage::EMBED_DIM;

    #[test]
    fn feature_hash_embedder_is_byte_identical() {
        // The default embedder MUST equal feature_hash_embed exactly, or every
        // vector test (rrf_proof, knn) would drift.
        let e = FeatureHashEmbedder::new(EMBED_DIM);
        for text in [
            "fn hello_world() {}",
            "load save cache queue bespoke",
            "",
            "!!! ... ,,,",
            "struct Goodbye {}",
        ] {
            assert_eq!(
                e.embed(text),
                feature_hash_embed(text, EMBED_DIM),
                "FeatureHashEmbedder must be byte-identical to feature_hash_embed for {text:?}"
            );
        }
        assert_eq!(e.id(), FEATURE_HASH_ID);
        assert_eq!(e.dim(), EMBED_DIM);
    }

    #[test]
    fn choice_none_is_plain_feature_hash() {
        // No model configured → plain feature-hash, no warning.
        assert_eq!(
            resolve_embedder_choice(None, |_| false),
            EmbedderChoice::FeatureHash
        );
        // Empty string is treated as "unset".
        assert_eq!(
            resolve_embedder_choice(Some("   "), |_| true),
            EmbedderChoice::FeatureHash
        );
    }

    #[test]
    fn choice_absent_model_falls_back_with_warning() {
        // A configured-but-MISSING model path → fallback decision + a warning
        // mentioning the path. No panic, no network (this is pure logic).
        let choice = resolve_embedder_choice(Some("/no/such/model.gguf"), |_| false);
        match choice {
            EmbedderChoice::FallbackToFeatureHash { warning } => {
                assert!(warning.contains("/no/such/model.gguf"));
                assert!(warning.contains("feature-hash"));
            }
            other => panic!("expected fallback, got {other:?}"),
        }
    }

    #[test]
    fn choice_present_model_selects_gguf() {
        // A configured-and-present path → select gguf (the loader decides success
        // later; with the feature off active_embedder maps this to a fallback).
        let choice = resolve_embedder_choice(Some("/tmp/real.gguf"), |_| true);
        assert_eq!(
            choice,
            EmbedderChoice::Gguf {
                model_path: "/tmp/real.gguf".to_string()
            }
        );
    }

    #[test]
    fn active_embedder_default_is_feature_hash() {
        // Regardless of feature flag, with no env configured the active embedder
        // is feature-hash of the requested dim — the no-model default.
        let e = active_embedder(EMBED_DIM);
        assert_eq!(e.id(), FEATURE_HASH_ID);
        assert_eq!(e.dim(), EMBED_DIM);
        // And it round-trips byte-identically.
        assert_eq!(
            e.embed("fn hello_world() {}"),
            feature_hash_embed("fn hello_world() {}", EMBED_DIM)
        );
    }
}
