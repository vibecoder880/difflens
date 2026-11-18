//! Deterministic fake embedder used by tests and model-less runs.
//!
//! Bag-of-character-trigrams plus word tokens, hashed into a fixed-size
//! vector with FNV-1a and L2-normalized. Cosine then behaves like a lexical
//! overlap score: identical texts score 1.0, near-duplicates score high,
//! unrelated texts score low — good enough for deterministic tests without
//! any model files or network access.

use super::{Embedder, l2_normalize};
use crate::error::CoreResult;

/// Default vector dimension (kept small so tests stay fast).
pub const DEFAULT_DIM: usize = 256;

/// Weight of whole-word token hits relative to trigram hits.
const WORD_WEIGHT: f32 = 2.0;

/// Deterministic hash embedder (test double, never loads a model).
#[derive(Debug, Clone)]
pub struct FakeHashEmbedder {
    dim: usize,
}

impl FakeHashEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(8) }
    }
}

impl Default for FakeHashEmbedder {
    fn default() -> Self {
        Self::new(DEFAULT_DIM)
    }
}

impl Embedder for FakeHashEmbedder {
    fn embed(&self, texts: &[&str]) -> CoreResult<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }

    fn name(&self) -> &'static str {
        "fake-hash"
    }
}

impl FakeHashEmbedder {
    fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        let lower = text.to_lowercase();

        for token in lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let idx = (fnv1a64(token.as_bytes()) as usize) % self.dim;
            v[idx] += WORD_WEIGHT;
        }

        let mut padded: Vec<char> = Vec::with_capacity(lower.chars().count() + 2);
        padded.push(' ');
        padded.extend(lower.chars());
        padded.push(' ');
        for window in padded.windows(3) {
            let mut bytes: Vec<u8> = Vec::with_capacity(12);
            for ch in window {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            let idx = (fnv1a64(&bytes) as usize) % self.dim;
            v[idx] += 1.0;
        }

        l2_normalize(&mut v);
        v
    }
}

/// FNV-1a 64-bit: stable across platforms and runs (no `DefaultHasher`
/// randomness anywhere).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_texts_have_unit_cosine() {
        let e = FakeHashEmbedder::default();
        let vs = e.embed(&["same text", "same text"]).unwrap();
        assert!((super::super::cosine(&vs[0], &vs[1]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_text_gives_zero_vector() {
        let e = FakeHashEmbedder::default();
        let vs = e.embed(&[""]).unwrap();
        assert!(vs[0].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn vietnamese_text_is_stable_across_calls() {
        let e = FakeHashEmbedder::default();
        let a = e.embed(&["Điều 5: Thanh toán trong vòng 30 ngày"]).unwrap();
        let b = e.embed(&["Điều 5: Thanh toán trong vòng 30 ngày"]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn word_overlap_dominates_trigram_noise() {
        let e = FakeHashEmbedder::default();
        let vs = e
            .embed(&[
                "termination of the agreement",
                "termination of this agreement",
                "xx yy zz qq ww",
            ])
            .unwrap();
        let cos = super::super::cosine;
        assert!(cos(&vs[0], &vs[1]) > cos(&vs[0], &vs[2]));
    }
}
