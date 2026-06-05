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
//! `gguf-embed` feature. It loads a USER-SUPPLIED local BERT checkpoint (a
//! safetensors `sentence-transformers/all-MiniLM-L6-v2` directory; never
//! vendored, never downloaded by us — see the loader doc) and runs a REAL
//! candle `BertModel` forward-pass + attention-masked mean-pool + L2-normalize
//! (US-1B). With the feature OFF, none of its ML dependencies
//! (candle/tokenizers) are pulled in and the model-weight scan stays 0 — the
//! `cargo tree -e normal` gate proves it.
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

/// Local-model embedder running a REAL candle `BertModel` forward-pass (US-1B).
///
/// ## What this is (honest status)
///
/// This struct OWNS the offline inference path. It loads a USER-SUPPLIED local
/// BERT checkpoint directory (a safetensors `all-MiniLM-L6-v2`: `config.json`,
/// `tokenizer.json`, `model.safetensors`) from disk with `std::fs` / mmap ONLY —
/// there is NO network code on this path and no `hf-hub` dependency (web research
/// 2026-06: candle and `safetensors` both load directly from a local path;
/// `hf-hub` is only the optional downloader, which we do not use). The checkpoint
/// is ALWAYS supplied by the user via [`EMBED_MODEL_ENV`]; we never vendor or
/// download it.
///
/// [`GgufEmbedder::embed`] tokenizes the text, runs the candle `BertModel`
/// forward-pass on the CPU, attention-masked-mean-pools the last hidden state,
/// and L2-normalizes — fully deterministic. `Device::Cpu` keeps the build
/// portable (no CUDA dep). Any load failure returns `Err` (never panics) so
/// [`load_gguf_or_fallback`] degrades to the feature-hash embedder with a stderr
/// warning; the zero-vector trap of v0.5 is gone because the real ctor only
/// succeeds for a genuine, dim-matching checkpoint.
#[cfg(feature = "gguf-embed")]
pub struct GgufEmbedder {
    id: String,
    dim: usize,
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
}

#[cfg(feature = "gguf-embed")]
impl std::fmt::Debug for GgufEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // BertModel / Tokenizer are large and not Debug-friendly; surface only the
        // stable identity so logs stay readable.
        f.debug_struct("GgufEmbedder")
            .field("id", &self.id)
            .field("dim", &self.dim)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "gguf-embed")]
impl GgufEmbedder {
    /// Load a user-supplied local BERT checkpoint from `model_path`. `model_path`
    /// may point at the `model.safetensors` FILE or at the checkpoint DIRECTORY;
    /// either way the directory is resolved and `config.json` / `tokenizer.json` /
    /// `model.safetensors` are read from it with `std::fs` / mmap — NO network, NO
    /// `hf-hub`.
    ///
    /// Returns `Err` (never panics) on any failure — a missing/empty file, a
    /// missing sibling, a parse error, or a `hidden_size != dim` mismatch (we
    /// REFUSE a model whose dimension does not match the index DDL width
    /// [`crate::storage::EMBED_DIM`]). The caller ([`load_gguf_or_fallback`]) then
    /// falls back to the feature-hash embedder with a stderr warning, so no
    /// degraded embedder is ever constructed.
    ///
    /// `id` is `gguf:<dir-stem>` so [`crate::schema::verify_embedder_meta`] refuses
    /// to mix this backend's vectors with the feature-hash ones (or a different
    /// checkpoint's) in the same index.
    pub fn load(model_path: &str, dim: usize) -> anyhow::Result<Self> {
        use anyhow::Context;
        use candle_core::DType;
        use candle_nn::VarBuilder;
        use candle_transformers::models::bert::{BertModel, Config};

        // Resolve the checkpoint DIRECTORY: accept either the .safetensors file or
        // the directory itself.
        let raw = std::path::Path::new(model_path);
        let dir: std::path::PathBuf = if raw.is_dir() {
            raw.to_path_buf()
        } else {
            raw.parent()
                .map(|p| p.to_path_buf())
                .with_context(|| format!("resolve checkpoint dir for '{model_path}'"))?
        };

        let config_path = dir.join("config.json");
        let tokenizer_path = dir.join("tokenizer.json");
        let weights_path = dir.join("model.safetensors");

        // config.json -> candle bert Config (serde_json).
        let config_file = std::fs::File::open(&config_path)
            .with_context(|| format!("open bert config '{}'", config_path.display()))?;
        let config: Config = serde_json::from_reader(std::io::BufReader::new(config_file))
            .with_context(|| format!("parse bert config '{}'", config_path.display()))?;

        // Refuse a model whose hidden_size does not match the index DDL width: a
        // mismatched-dim checkpoint would write rows vec0 cannot store / query.
        anyhow::ensure!(
            config.hidden_size == dim,
            "embedding model '{model_path}' has hidden_size {} but the index requires dim {dim}; \
             refusing to load a mismatched-dim model",
            config.hidden_size
        );

        // tokenizer.json -> tokenizers::Tokenizer.
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            anyhow::anyhow!("load tokenizer '{}': {e}", tokenizer_path.display())
        })?;

        // model.safetensors -> VarBuilder (mmaped, CPU, F32).
        let device = candle_core::Device::Cpu;
        // SAFETY: from_mmaped_safetensors mmaps the file read-only; the path is a
        // user-supplied local checkpoint, read with no network.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path.clone()], DType::F32, &device)
                .with_context(|| {
                    format!("mmap safetensors weights '{}'", weights_path.display())
                })?
        };
        let model = BertModel::load(vb, &config)
            .map_err(|e| anyhow::anyhow!("build BertModel from '{}': {e}", weights_path.display()))?;

        let id = format!("gguf:{}", dir_stem(&dir));
        Ok(Self {
            id,
            dim,
            model,
            tokenizer,
            device,
        })
    }

    /// Run the candle BERT forward-pass for `text` and return the
    /// attention-masked mean-pooled, L2-normalized embedding. Returns `Err` on any
    /// tensor/tokenizer failure so [`GgufEmbedder::embed`] can degrade to a
    /// zero-free deterministic fallback without panicking.
    fn embed_inner(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        use candle_core::{DType, Tensor};

        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        let ids: Vec<u32> = encoding.get_ids().to_vec();
        let mask: Vec<u32> = encoding.get_attention_mask().to_vec();
        let n = ids.len();

        // [1, n] batched tensors. token_type_ids are all-zero (single-segment).
        let input_ids = Tensor::new(ids.as_slice(), &self.device)?.unsqueeze(0)?;
        let attention_mask = Tensor::new(mask.as_slice(), &self.device)?.unsqueeze(0)?;
        let token_type_ids = Tensor::zeros((1, n), DType::U32, &self.device)?;

        // last_hidden_state: [1, n, hidden_size].
        let hidden = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))?;

        // Attention-masked mean-pool over tokens, then L2-normalize — the canonical
        // sentence-transformers pooling (matches the candle bert example).
        let mask_f = attention_mask.to_dtype(DType::F32)?.unsqueeze(2)?; // [1, n, 1]
        let sum_mask = mask_f.sum(1)?; // [1, 1]
        let pooled = hidden.broadcast_mul(&mask_f)?.sum(1)?; // [1, hidden_size]
        let pooled = pooled.broadcast_div(&sum_mask)?;
        let normed = pooled.broadcast_div(&pooled.sqr()?.sum_keepdim(1)?.sqrt()?)?;

        let v: Vec<f32> = normed.squeeze(0)?.to_vec1()?;
        anyhow::ensure!(
            v.len() == self.dim,
            "embedding dim {} != expected {}",
            v.len(),
            self.dim
        );
        Ok(v)
    }
}

#[cfg(feature = "gguf-embed")]
impl Embedder for GgufEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        // Never panic (the Embedder contract). On the rare tensor error, fall back
        // to the deterministic feature-hash vector so a query still returns a
        // usable, non-zero vector rather than crashing the indexer/server.
        match self.embed_inner(text) {
            Ok(v) => v,
            Err(err) => {
                eprintln!(
                    "apohara-codesearch: gguf embed failed for one input ({err}); \
                     using feature-hash for this text only"
                );
                crate::embeddings::feature_hash_embed(text, self.dim)
            }
        }
    }
    fn id(&self) -> &str {
        &self.id
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

/// Directory stem (final path component) used to tag a gguf embedder's id.
#[cfg(feature = "gguf-embed")]
fn dir_stem(path: &std::path::Path) -> String {
    path.file_name()
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

    /// US-1B real round-trip: with the `gguf-embed` feature ON and the local
    /// MiniLM-L6 checkpoint present, the embedder loads, runs the candle BERT
    /// forward-pass, and produces a sane sentence embedding. Skips gracefully when
    /// the model dir is absent (so CI without the checkpoint still passes) — no
    /// network, ever.
    #[cfg(feature = "gguf-embed")]
    #[test]
    fn gguf_embed_real_round_trip() {
        // Where the one-time, OUTSIDE-the-repo checkpoint lives. Overridable so the
        // test follows a relocated model, but never vendored.
        let dir = std::env::var(EMBED_MODEL_ENV).unwrap_or_else(|_| {
            "/home/thelinconx/apohara-models/all-MiniLM-L6-v2".to_string()
        });
        if !std::path::Path::new(&dir).join("model.safetensors").exists() {
            eprintln!("skipping gguf_embed_real_round_trip: checkpoint not found at {dir}");
            return;
        }

        let e = GgufEmbedder::load(&dir, EMBED_DIM)
            .expect("the real MiniLM-L6 checkpoint must load under --features gguf-embed");
        assert_eq!(e.dim(), EMBED_DIM);
        assert!(e.id().starts_with("gguf:"), "id must be gguf:<stem>, got {}", e.id());

        let cat = e.embed("the cat sat on the mat");

        // NOT all-zero.
        assert!(
            cat.iter().any(|&x| x != 0.0),
            "real embed must not be an all-zero vector"
        );
        // L2-normalized: ||v|| ~= 1.0.
        let norm = cat.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "embedding must be L2-normalized, got ||v|| = {norm}"
        );
        // Deterministic: two calls byte-identical.
        assert_eq!(
            cat,
            e.embed("the cat sat on the mat"),
            "gguf embed must be deterministic"
        );

        // Semantic signal: a paraphrase must be closer than an unrelated sentence.
        let paraphrase = e.embed("a feline rested on the rug");
        let unrelated = e.embed("compile the rust binary");
        let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let cos_para = cos(&cat, &paraphrase);
        let cos_unrel = cos(&cat, &unrelated);
        assert!(
            cos_para > cos_unrel,
            "paraphrase cos {cos_para} must exceed unrelated cos {cos_unrel}"
        );
    }

    /// A MISSING model path under `--features gguf-embed` must STILL fall back to
    /// the feature-hash embedder (no panic, no network), proving the loader degrades
    /// gracefully when the user-configured checkpoint is absent.
    #[cfg(feature = "gguf-embed")]
    #[test]
    fn gguf_embed_missing_model_falls_back() {
        let missing = "/no/such/checkpoint/all-MiniLM-L6-v2";
        // load() returns Err for an absent checkpoint (config.json open fails).
        assert!(
            GgufEmbedder::load(missing, EMBED_DIM).is_err(),
            "absent checkpoint must error, not panic"
        );
        // The active fallback path routes to feature-hash, never a gguf:* embedder.
        let e = load_gguf_or_fallback(missing, EMBED_DIM);
        assert_eq!(
            e.id(),
            FEATURE_HASH_ID,
            "missing model must fall back to feature-hash, not gguf:*"
        );
        let v = e.embed("anything");
        assert_eq!(
            v,
            feature_hash_embed("anything", EMBED_DIM),
            "fallback embed must be byte-identical to feature-hash"
        );
        assert!(
            v.iter().any(|&x| x != 0.0),
            "embed must never be an all-zero vector"
        );
    }
}
