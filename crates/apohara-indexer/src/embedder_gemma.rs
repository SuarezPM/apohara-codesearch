// SPDX-License-Identifier: MIT OR Apache-2.0

//! EmbeddingGemma (Gemma3TextModel, bidirectional encoder) embedder — a REAL
//! pure-candle forward-pass, compiled ONLY behind the `gguf-embed` feature.
//!
//! ## Provenance (do not "improve" the numerics)
//!
//! This is a faithful port of a validated spike that reached cosine `0.999983`
//! against the ONNX F32 reference (`sentence_embedding`). Every numerical detail
//! below was the result of that parity work and MUST stay byte-for-byte
//! equivalent: RoPE DUAL per layer type (sliding layers `theta=10000`, full
//! layers at indices 5/11/17/23 `theta=1000000`), embedding scale `*sqrt(768)`,
//! Gemma RMSNorm = `x_normed*(1+weight)` in f32, QK-norm RMSNorm over
//! `head_dim=256` BEFORE RoPE, query scaling `256^-0.5`, GeGLU
//! `down(gelu_tanh(gate)*up)`, four RMSNorms per layer with the Gemma2/3 residual
//! pattern, BIDIRECTIONAL attention (no causal mask), then
//! transformer → masked mean-pool (include_prompt) → 2_Dense (768→3072) →
//! 3_Dense (3072→768) → L2-normalize.
//!
//! ## Loading (offline only)
//!
//! Weights are USER-SUPPLIED safetensors read with `std::fs` / mmap ONLY — no
//! network, no `hf-hub`. The model directory must contain `model.safetensors`,
//! `2_Dense/model.safetensors`, `3_Dense/model.safetensors` and `tokenizer.json`.
//!
//! ## Asymmetric prompts + Matryoshka (256d)
//!
//! The model is asymmetric: queries are prefixed
//! `"task: code retrieval | query: "`, documents `"title: none | text: "`. The
//! native output is 768d; we truncate to the first 256 dims (Matryoshka) and
//! re-L2-normalize, so [`EmbeddingGemmaEmbedder::dim`] is 256. The internal
//! parity test validates the FULL 768d (pre-truncation) against the reference.

use anyhow::{Context, Result};
use candle_core::{DType, Device, IndexOp, Tensor, D};
use candle_nn::{Linear, Module, VarBuilder};
use tokenizers::Tokenizer;

use crate::embedder::Embedder;

// --- Gemma3-300m (EmbeddingGemma) architecture constants (fixed checkpoint) ---
const HIDDEN_SIZE: usize = 768;
const NUM_LAYERS: usize = 24;
const NUM_HEADS: usize = 3;
const NUM_KV_HEADS: usize = 1;
const HEAD_DIM: usize = 256;
const INTERMEDIATE_SIZE: usize = 1152;
const VOCAB_SIZE: usize = 262144;
const RMS_EPS: f64 = 1e-6;
const ROPE_THETA_GLOBAL: f64 = 1_000_000.0; // full_attention layers
const ROPE_THETA_LOCAL: f64 = 10_000.0; // sliding_attention layers (rope_local_base_freq)
const QUERY_PRE_ATTN_SCALAR: f64 = 256.0;

/// Native (pre-Matryoshka) output width. The parity check validates this width.
pub const NATIVE_DIM: usize = 768;
/// Matryoshka-truncated output width — what [`Embedder::dim`] reports.
pub const MATRYOSHKA_DIM: usize = 256;

/// Stable backend id; includes the truncated dim so [`crate::schema::verify_embedder_meta`]
/// refuses to mix it with the feature-hash (or a differently-sized) index.
pub const EMBEDDINGGEMMA_ID: &str = "embeddinggemma-300m-256";

/// Document prefix prepended by [`Embedder::embed_document`] (asymmetric model).
pub const DOC_PROMPT: &str = "title: none | text: ";
/// Query prefix prepended by [`Embedder::embed_query`] (asymmetric model).
pub const QUERY_PROMPT: &str = "task: code retrieval | query: ";

// layer_types from config.json: full_attention at indices 5,11,17,23; rest sliding.
fn is_full_attention(layer_idx: usize) -> bool {
    matches!(layer_idx, 5 | 11 | 17 | 23)
}

/// Gemma RMSNorm: output = x_normed * (1 + weight), computed in f32.
struct RmsNorm {
    weight: Tensor,
}

impl RmsNorm {
    fn load(vb: VarBuilder, dim: usize) -> Result<Self> {
        let weight = vb.get(dim, "weight")?;
        Ok(Self { weight })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let in_dtype = x.dtype();
        let x = x.to_dtype(DType::F32)?;
        let variance = x.sqr()?.mean_keepdim(D::Minus1)?;
        let x_normed = x.broadcast_div(&(variance + RMS_EPS)?.sqrt()?)?;
        let w = (&self.weight.to_dtype(DType::F32)? + 1.0)?;
        let out = x_normed.broadcast_mul(&w)?;
        Ok(out.to_dtype(in_dtype)?)
    }
}

fn linear_no_bias(vb: VarBuilder, in_dim: usize, out_dim: usize) -> Result<Linear> {
    let w = vb.get((out_dim, in_dim), "weight")?;
    Ok(Linear::new(w, None))
}

struct RotaryEmbedding {
    cos: Tensor,
    sin: Tensor,
}

impl RotaryEmbedding {
    fn new(seq_len: usize, theta: f64, device: &Device) -> Result<Self> {
        let half = HEAD_DIM / 2;
        let inv_freq: Vec<f32> = (0..half)
            .map(|i| 1f32 / (theta as f32).powf(2.0 * i as f32 / HEAD_DIM as f32))
            .collect();
        let inv_freq = Tensor::from_vec(inv_freq, (1, half), device)?;
        let t: Vec<f32> = (0..seq_len).map(|i| i as f32).collect();
        let t = Tensor::from_vec(t, (seq_len, 1), device)?;
        let freqs = t.broadcast_mul(&inv_freq)?; // [seq, half]
        Ok(Self {
            cos: freqs.cos()?,
            sin: freqs.sin()?,
        })
    }

    /// Apply rotary to x: [b, heads, seq, head_dim] using the rotate_half convention.
    fn apply(&self, x: &Tensor) -> Result<Tensor> {
        let (_b, _h, seq, _d) = x.dims4()?;
        let cos = self.cos.narrow(0, 0, seq)?;
        let sin = self.sin.narrow(0, 0, seq)?;
        let cos = Tensor::cat(&[&cos, &cos], D::Minus1)?; // [seq, head_dim]
        let sin = Tensor::cat(&[&sin, &sin], D::Minus1)?;
        let cos = cos.reshape((1, 1, seq, HEAD_DIM))?;
        let sin = sin.reshape((1, 1, seq, HEAD_DIM))?;
        let rotated = rotate_half(x)?;
        let out = (x.broadcast_mul(&cos)? + rotated.broadcast_mul(&sin)?)?;
        Ok(out)
    }
}

fn rotate_half(x: &Tensor) -> Result<Tensor> {
    let last = x.dim(D::Minus1)?;
    let half = last / 2;
    let x1 = x.narrow(D::Minus1, 0, half)?;
    let x2 = x.narrow(D::Minus1, half, half)?;
    Ok(Tensor::cat(&[&x2.neg()?, &x1], D::Minus1)?)
}

struct Attention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    o_proj: Linear,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
}

impl Attention {
    fn load(vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            q_proj: linear_no_bias(vb.pp("q_proj"), HIDDEN_SIZE, NUM_HEADS * HEAD_DIM)?,
            k_proj: linear_no_bias(vb.pp("k_proj"), HIDDEN_SIZE, NUM_KV_HEADS * HEAD_DIM)?,
            v_proj: linear_no_bias(vb.pp("v_proj"), HIDDEN_SIZE, NUM_KV_HEADS * HEAD_DIM)?,
            o_proj: linear_no_bias(vb.pp("o_proj"), NUM_HEADS * HEAD_DIM, HIDDEN_SIZE)?,
            q_norm: RmsNorm::load(vb.pp("q_norm"), HEAD_DIM)?,
            k_norm: RmsNorm::load(vb.pp("k_norm"), HEAD_DIM)?,
        })
    }

    /// x: [b, seq, hidden]; bidirectional, no causal mask.
    fn forward(&self, x: &Tensor, rope: &RotaryEmbedding) -> Result<Tensor> {
        let (b, seq, _) = x.dims3()?;
        let q = self.q_proj.forward(x)?;
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;

        let q = q.reshape((b, seq, NUM_HEADS, HEAD_DIM))?.transpose(1, 2)?;
        let k = k
            .reshape((b, seq, NUM_KV_HEADS, HEAD_DIM))?
            .transpose(1, 2)?;
        let v = v
            .reshape((b, seq, NUM_KV_HEADS, HEAD_DIM))?
            .transpose(1, 2)?;

        // QK-norm over head_dim (RMSNorm) BEFORE RoPE.
        let q = self.q_norm.forward(&q)?;
        let k = self.k_norm.forward(&k)?;

        // RoPE.
        let q = rope.apply(&q)?;
        let k = rope.apply(&k)?;

        // GQA: repeat kv heads to match query heads.
        let rep = NUM_HEADS / NUM_KV_HEADS;
        let k = repeat_kv(&k, rep)?;
        let v = repeat_kv(&v, rep)?;

        // Query scaling: query_pre_attn_scalar^-0.5.
        let scale = QUERY_PRE_ATTN_SCALAR.powf(-0.5);
        let q = (q * scale)?;

        let q = q.contiguous()?;
        let k = k.contiguous()?;
        let v = v.contiguous()?;

        // attn = softmax(q @ k^T) @ v (full bidirectional, no mask).
        let attn = q.matmul(&k.transpose(2, 3)?)?;
        let attn = candle_nn::ops::softmax(&attn.to_dtype(DType::F32)?, D::Minus1)?;
        let out = attn.matmul(&v.to_dtype(DType::F32)?)?;

        let out = out
            .transpose(1, 2)?
            .reshape((b, seq, NUM_HEADS * HEAD_DIM))?
            .to_dtype(x.dtype())?;
        Ok(self.o_proj.forward(&out)?)
    }
}

fn repeat_kv(x: &Tensor, rep: usize) -> Result<Tensor> {
    if rep == 1 {
        return Ok(x.clone());
    }
    let (b, kv, seq, hd) = x.dims4()?;
    let x = x
        .unsqueeze(2)?
        .expand((b, kv, rep, seq, hd))?
        .reshape((b, kv * rep, seq, hd))?;
    Ok(x)
}

struct Mlp {
    gate_proj: Linear,
    up_proj: Linear,
    down_proj: Linear,
}

impl Mlp {
    fn load(vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            gate_proj: linear_no_bias(vb.pp("gate_proj"), HIDDEN_SIZE, INTERMEDIATE_SIZE)?,
            up_proj: linear_no_bias(vb.pp("up_proj"), HIDDEN_SIZE, INTERMEDIATE_SIZE)?,
            down_proj: linear_no_bias(vb.pp("down_proj"), INTERMEDIATE_SIZE, HIDDEN_SIZE)?,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        // GeGLU: down(gelu_tanh(gate) * up). candle's .gelu() is the tanh approximation.
        let gate = self.gate_proj.forward(x)?.gelu()?;
        let up = self.up_proj.forward(x)?;
        let h = (gate * up)?;
        Ok(self.down_proj.forward(&h)?)
    }
}

struct Layer {
    input_ln: RmsNorm,
    post_attn_ln: RmsNorm,
    pre_ff_ln: RmsNorm,
    post_ff_ln: RmsNorm,
    attn: Attention,
    mlp: Mlp,
}

impl Layer {
    fn load(vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            input_ln: RmsNorm::load(vb.pp("input_layernorm"), HIDDEN_SIZE)?,
            post_attn_ln: RmsNorm::load(vb.pp("post_attention_layernorm"), HIDDEN_SIZE)?,
            pre_ff_ln: RmsNorm::load(vb.pp("pre_feedforward_layernorm"), HIDDEN_SIZE)?,
            post_ff_ln: RmsNorm::load(vb.pp("post_feedforward_layernorm"), HIDDEN_SIZE)?,
            attn: Attention::load(vb.pp("self_attn"))?,
            mlp: Mlp::load(vb.pp("mlp"))?,
        })
    }

    /// Gemma2/3 residual pattern:
    ///   h   = x + post_attn_ln(attn(input_ln(x)))
    ///   out = h + post_ff_ln(mlp(pre_ff_ln(h)))
    fn forward(&self, x: &Tensor, rope: &RotaryEmbedding) -> Result<Tensor> {
        let normed = self.input_ln.forward(x)?;
        let attn_out = self.attn.forward(&normed, rope)?;
        let attn_out = self.post_attn_ln.forward(&attn_out)?;
        let h = (x + attn_out)?;

        let normed = self.pre_ff_ln.forward(&h)?;
        let mlp_out = self.mlp.forward(&normed)?;
        let mlp_out = self.post_ff_ln.forward(&mlp_out)?;
        Ok((h + mlp_out)?)
    }
}

struct Model {
    embed_tokens: Tensor,
    layers: Vec<Layer>,
    norm: RmsNorm,
    dense2: Linear, // 768 -> 3072
    dense3: Linear, // 3072 -> 768
    device: Device,
}

impl Model {
    fn load(dir: &std::path::Path, device: &Device) -> Result<Self> {
        let weights = dir.join("model.safetensors");
        let dense2_w = dir.join("2_Dense").join("model.safetensors");
        let dense3_w = dir.join("3_Dense").join("model.safetensors");

        // SAFETY: from_mmaped_safetensors mmaps read-only; paths are user-supplied
        // local checkpoints, read with no network.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(std::slice::from_ref(&weights), DType::F32, device)
                .with_context(|| format!("mmap safetensors '{}'", weights.display()))?
        };
        let embed_tokens = vb.get((VOCAB_SIZE, HIDDEN_SIZE), "embed_tokens.weight")?;
        let mut layers = Vec::with_capacity(NUM_LAYERS);
        for i in 0..NUM_LAYERS {
            layers.push(Layer::load(vb.pp(format!("layers.{i}")))?);
        }
        let norm = RmsNorm::load(vb.pp("norm"), HIDDEN_SIZE)?;

        let vb2 = unsafe {
            VarBuilder::from_mmaped_safetensors(std::slice::from_ref(&dense2_w), DType::F32, device)
                .with_context(|| format!("mmap safetensors '{}'", dense2_w.display()))?
        };
        let dense2 = linear_no_bias(vb2.pp("linear"), 768, 3072)?;

        let vb3 = unsafe {
            VarBuilder::from_mmaped_safetensors(std::slice::from_ref(&dense3_w), DType::F32, device)
                .with_context(|| format!("mmap safetensors '{}'", dense3_w.display()))?
        };
        let dense3 = linear_no_bias(vb3.pp("linear"), 3072, 768)?;

        Ok(Self {
            embed_tokens,
            layers,
            norm,
            dense2,
            dense3,
            device: device.clone(),
        })
    }

    /// Run the full transformer + dense head and return the native 768d
    /// L2-normalized sentence embedding (the parity-validated output).
    fn forward_native(&self, token_ids: &[u32]) -> Result<Vec<f32>> {
        let seq = token_ids.len();
        let ids = Tensor::from_vec(token_ids.to_vec(), (seq,), &self.device)?;
        let mut x = self.embed_tokens.index_select(&ids, 0)?;
        // Embedding scaling: * sqrt(hidden_size).
        let scale = (HIDDEN_SIZE as f64).sqrt();
        x = (x * scale)?;
        let mut x = x.unsqueeze(0)?; // [1, seq, hidden]

        let rope_global = RotaryEmbedding::new(seq, ROPE_THETA_GLOBAL, &self.device)?;
        let rope_local = RotaryEmbedding::new(seq, ROPE_THETA_LOCAL, &self.device)?;
        for (idx, layer) in self.layers.iter().enumerate() {
            let rope = if is_full_attention(idx) {
                &rope_global
            } else {
                &rope_local
            };
            x = layer.forward(&x, rope)?;
        }
        x = self.norm.forward(&x)?; // [1, seq, hidden]

        // mean-pool over tokens (include_prompt=true, mask all ones).
        let pooled = x.i(0)?.mean(0)?; // [hidden]
        let pooled = pooled.unsqueeze(0)?; // [1, hidden]

        // 2_Dense -> 3_Dense.
        let h = self.dense2.forward(&pooled)?;
        let h = self.dense3.forward(&h)?;

        // L2-normalize.
        let h = h.i(0)?; // [768]
        let norm = h.sqr()?.sum_all()?.sqrt()?.to_scalar::<f32>()?;
        let h = (h / (norm as f64 + 1e-12))?;
        Ok(h.to_vec1::<f32>()?)
    }
}

/// REAL EmbeddingGemma embedder (pure-candle Gemma3 forward-pass). Outputs a
/// 256d Matryoshka-truncated, re-L2-normalized vector with asymmetric prompts.
pub struct EmbeddingGemmaEmbedder {
    id: String,
    model: Model,
    tokenizer: Tokenizer,
}

impl std::fmt::Debug for EmbeddingGemmaEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingGemmaEmbedder")
            .field("id", &self.id)
            .field("dim", &MATRYOSHKA_DIM)
            .finish_non_exhaustive()
    }
}

impl EmbeddingGemmaEmbedder {
    /// Load a user-supplied EmbeddingGemma checkpoint directory (must contain
    /// `model.safetensors`, `2_Dense/model.safetensors`, `3_Dense/model.safetensors`
    /// and `tokenizer.json`). `std::fs` / mmap ONLY — no network, never panics.
    ///
    /// `model_path` may be the directory or any file inside it.
    pub fn load(model_path: &str) -> Result<Self> {
        let raw = std::path::Path::new(model_path);
        let dir: std::path::PathBuf = if raw.is_dir() {
            raw.to_path_buf()
        } else {
            raw.parent()
                .map(|p| p.to_path_buf())
                .with_context(|| format!("resolve checkpoint dir for '{model_path}'"))?
        };

        let tokenizer_path = dir.join("tokenizer.json");
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("load tokenizer '{}': {e}", tokenizer_path.display()))?;

        let device = Device::Cpu;
        let model = Model::load(&dir, &device)?;

        Ok(Self {
            id: EMBEDDINGGEMMA_ID.to_string(),
            model,
            tokenizer,
        })
    }

    /// True when `dir` looks like an EmbeddingGemma checkpoint (`config.json` with
    /// `model_type == "gemma3_text"`). Used by the loader to route between the
    /// BERT and EmbeddingGemma backends.
    pub fn is_gemma_checkpoint(dir: &std::path::Path) -> bool {
        let config = dir.join("config.json");
        let Ok(text) = std::fs::read_to_string(&config) else {
            return false;
        };
        let Ok(value): Result<serde_json::Value, _> = serde_json::from_str(&text) else {
            return false;
        };
        value
            .get("model_type")
            .and_then(|v| v.as_str())
            .map(|s| s == "gemma3_text")
            .unwrap_or(false)
    }

    /// Native 768d embedding for `full_text` (caller supplies the full prompt).
    /// Exposed for the parity test, which validates the pre-truncation width.
    pub fn embed_native(&self, full_text: &str) -> Result<Vec<f32>> {
        let enc = self
            .tokenizer
            .encode(full_text, true)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        self.model.forward_native(enc.get_ids())
    }

    /// Embed `full_text` (already prompt-prefixed) and Matryoshka-truncate to
    /// [`MATRYOSHKA_DIM`] with a fresh L2 re-normalization.
    fn embed_truncated(&self, full_text: &str) -> Result<Vec<f32>> {
        let mut v = self.embed_native(full_text)?;
        v.truncate(MATRYOSHKA_DIM);
        l2_normalize(&mut v);
        Ok(v)
    }
}

/// Cosine similarity of two equal-length vectors.
#[cfg(test)]
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb + 1e-12)
}

/// In-place L2 normalization (matches the native head's final step).
fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

impl Embedder for EmbeddingGemmaEmbedder {
    /// Generic embed uses the DOCUMENT prompt (the indexing default). Query
    /// call-sites use [`Embedder::embed_query`] explicitly.
    fn embed(&self, text: &str) -> Vec<f32> {
        self.embed_document(text)
    }

    fn embed_query(&self, text: &str) -> Vec<f32> {
        let full = format!("{QUERY_PROMPT}{text}");
        self.embed_or_zero(&full)
    }

    fn embed_document(&self, text: &str) -> Vec<f32> {
        let full = format!("{DOC_PROMPT}{text}");
        self.embed_or_zero(&full)
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn dim(&self) -> usize {
        MATRYOSHKA_DIM
    }
}

impl EmbeddingGemmaEmbedder {
    /// Never panic (the [`Embedder`] contract). On a rare tensor error, fall back
    /// to a deterministic feature-hash vector of the same width so a query still
    /// returns a usable, non-zero vector rather than crashing.
    fn embed_or_zero(&self, full_text: &str) -> Vec<f32> {
        match self.embed_truncated(full_text) {
            Ok(v) => v,
            Err(err) => {
                eprintln!(
                    "apohara-codesearch: embeddinggemma embed failed for one input ({err}); \
                     using feature-hash for this text only"
                );
                crate::embeddings::feature_hash_embed(full_text, MATRYOSHKA_DIM)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::EMBED_MODEL_ENV;

    /// Default model dir for the parity test (overridable via `APOHARA_EMBED_MODEL`).
    const TEST_MODEL_DIR: &str = "/tmp/eg-st";
    /// The tokenizer + reference oracle from the validated spike.
    const TEST_TOKENIZER: &str = "/tmp/eg-onnx/tokenizer.json";
    const TEST_REFERENCE: &str = "/tmp/eg-onnx/embeddinggemma-reference.json";

    #[derive(serde::Deserialize)]
    struct Reference {
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        prompt: String,
        text: String,
        vec: Vec<f32>,
    }

    fn model_dir() -> String {
        std::env::var(EMBED_MODEL_ENV).unwrap_or_else(|_| TEST_MODEL_DIR.to_string())
    }

    /// Identity gate: the integrated EmbeddingGemma backend reports the
    /// Matryoshka dim (256) and a dim-tagged id, with NO model and NO ML load.
    #[test]
    fn matryoshka_constants_and_id() {
        assert_eq!(MATRYOSHKA_DIM, 256);
        assert_eq!(NATIVE_DIM, 768);
        assert!(EMBEDDINGGEMMA_ID.contains("256"), "id must encode the dim");
        assert_eq!(QUERY_PROMPT, "task: code retrieval | query: ");
        assert_eq!(DOC_PROMPT, "title: none | text: ");
    }

    /// Parity test (gated by model presence, like the gguf-embed BERT tests).
    ///
    /// Loads the INTEGRATED [`EmbeddingGemmaEmbedder`] and asserts the FULL 768d
    /// (pre-Matryoshka) output reaches cosine >= 0.999 vs the ONNX F32 oracle for
    /// all 12 reference cases. Skips gracefully (no failure) when the model or the
    /// oracle files are absent, so CI without the checkpoint still passes — no
    /// network, ever.
    #[test]
    fn embeddinggemma_parity_768() {
        let dir = model_dir();
        let weights = std::path::Path::new(&dir).join("model.safetensors");
        if !weights.exists()
            || !std::path::Path::new(TEST_REFERENCE).exists()
            || !std::path::Path::new(TEST_TOKENIZER).exists()
        {
            eprintln!(
                "skipping embeddinggemma_parity_768: checkpoint/oracle not found \
                 (model dir '{dir}', reference '{TEST_REFERENCE}')"
            );
            return;
        }

        // The model dir lacks tokenizer.json in the spike layout (it lives under
        // /tmp/eg-onnx); load the embedder against a dir we know has both, by
        // pointing the embedder loader at the model dir and overriding the
        // tokenizer through a copy is overkill — instead load the model and use a
        // standalone tokenizer for full control of the prompt-prefixed input.
        let embedder = EmbeddingGemmaEmbedder::load(&dir)
            .or_else(|_| {
                // Fallback: the model dir has no tokenizer.json; build the embedder
                // manually with the oracle's tokenizer.
                let device = Device::Cpu;
                let model = Model::load(std::path::Path::new(&dir), &device)?;
                let tokenizer = Tokenizer::from_file(TEST_TOKENIZER)
                    .map_err(|e| anyhow::anyhow!("load tokenizer: {e}"))?;
                anyhow::Ok(EmbeddingGemmaEmbedder {
                    id: EMBEDDINGGEMMA_ID.to_string(),
                    model,
                    tokenizer,
                })
            })
            .expect("EmbeddingGemma checkpoint must load under --features gguf-embed");

        assert_eq!(embedder.dim(), MATRYOSHKA_DIM);
        assert!(embedder.id().starts_with("embeddinggemma"));

        let reference: Reference =
            serde_json::from_str(&std::fs::read_to_string(TEST_REFERENCE).unwrap()).unwrap();

        let mut min_cos = f32::INFINITY;
        for case in &reference.cases {
            assert_eq!(case.vec.len(), NATIVE_DIM, "reference must be 768d");
            let full = format!("{}{}", case.prompt, case.text);
            // Validate the PRE-truncation 768d output against the 768d oracle.
            let out = embedder.embed_native(&full).unwrap();
            assert_eq!(out.len(), NATIVE_DIM);
            let cos = cosine(&out, &case.vec);
            if cos < min_cos {
                min_cos = cos;
            }
        }
        eprintln!("embeddinggemma_parity_768: min cosine = {min_cos:.6}");
        assert!(
            min_cos >= 0.999,
            "integrated EmbeddingGemma parity must stay >= 0.999, got {min_cos:.6}"
        );

        // Asymmetric prompts produce DIFFERENT vectors for the same text, and the
        // public Embedder methods return the 256d Matryoshka width.
        let q = embedder.embed_query("sort an array");
        let d = embedder.embed_document("sort an array");
        assert_eq!(q.len(), MATRYOSHKA_DIM);
        assert_eq!(d.len(), MATRYOSHKA_DIM);
        assert!(
            cosine(&q, &d) < 0.9999,
            "asymmetric prompts must differ for the same text"
        );
        // 256d Matryoshka output stays L2-normalized.
        let n: f32 = q.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-3, "256d output must be L2-normalized");
    }
}
