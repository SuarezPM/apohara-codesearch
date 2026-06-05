//! Deterministic feature-hashing embeddings (blake3-based).
//!
//! Replaces the previous Nomic BERT loader (G8.A.1 deleted the candle deps;
//! G8.A.3 deletes the loader code). Quality is below transformer-based
//! embeddings but adequate for code search MVP, and eliminates the OOM hazard
//! from in-process model load. Deterministic + ~0 RAM + no model download.

use blake3::Hasher;

/// Produce a feature-hashed embedding of `text` with `dim` buckets.
///
/// Pipeline:
///   1. Lower-case, then split on any non-alphanumeric character (including `_`,
///      which makes `hello_world` collide with `hello world` — desirable for
///      code search where snake_case identifiers should match natural-language
///      queries).
///   2. For each token, blake3-hash it. Use bytes [0..4) as the bucket index
///      and bit 0 of byte 4 as the +/-1 sign (signed feature hashing — reduces
///      collision bias vs. always-positive hashing).
///   3. L2-normalize so vec0's L2 distance / cosine give equivalent rankings.
pub fn feature_hash_embed(text: &str, dim: usize) -> Vec<f32> {
    let mut vec = vec![0f32; dim];
    let lowered = text.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return vec;
    }
    for tok in &tokens {
        let mut hasher = Hasher::new();
        hasher.update(tok.as_bytes());
        let hash = hasher.finalize();
        let bytes = hash.as_bytes();
        let bucket = (u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize) % dim;
        let sign = if bytes[4] & 1 == 0 { 1.0 } else { -1.0 };
        vec[bucket] += sign;
    }
    let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
    vec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_same_input_same_output() {
        let a = feature_hash_embed("hello world", 384);
        let b = feature_hash_embed("hello world", 384);
        assert_eq!(a, b);
    }

    #[test]
    fn different_inputs_different_outputs() {
        let a = feature_hash_embed("hello world", 384);
        let b = feature_hash_embed("goodbye moon", 384);
        assert_ne!(a, b);
    }

    #[test]
    fn unit_norm() {
        let v = feature_hash_embed("some code here fn foo bar", 384);
        let norm: f32 = v.iter().map(|f| f * f).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5 || norm == 0.0,
            "expected unit norm, got {}",
            norm
        );
    }

    #[test]
    fn semantic_nn_smoke() {
        // The G8.A.2 contract test asserts 'hello world function' is closer to
        // 'fn hello_world() {}' than 'struct Goodbye {}'. Tokens after underscore
        // split: query={hello,world,function}, A={fn,hello,world}, B={struct,goodbye}.
        // A shares 2 tokens with the query; B shares 0. Cosine should reflect that.
        let q = feature_hash_embed("hello world function", 384);
        let a = feature_hash_embed("fn hello_world() {}", 384);
        let b = feature_hash_embed("struct Goodbye {}", 384);
        let dot_a: f32 = q.iter().zip(a.iter()).map(|(x, y)| x * y).sum();
        let dot_b: f32 = q.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        assert!(
            dot_a > dot_b,
            "expected closer to chunk_a (dot_a={}, dot_b={})",
            dot_a,
            dot_b
        );
    }
}
