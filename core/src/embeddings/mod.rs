//! Embedding abstraction for the semantic alignment pass.
//!
//! [`Embedder`] turns block texts into fixed-size L2-normalized vectors so
//! the aligner can compute cosine similarities. Two implementations ship:
//!
//! - [`fake::FakeHashEmbedder`] — deterministic hash-based vectors, always
//!   compiled. Tests and model-less runs use it; no network, no files.
//! - [`ort_e5::OrtE5Embedder`] — real multilingual-e5-small ONNX inference
//!   through `ort`, behind the `embeddings-ort` feature.
//!
//! 100% local: no network access anywhere in this module. Model files are
//! fetched by `scripts/fetch-models.sh` (the only network egress for
//! embeddings in the whole repository).

pub mod fake;

#[cfg(feature = "embeddings-ort")]
pub mod ort_e5;

pub use fake::FakeHashEmbedder;

use crate::error::CoreResult;

/// Turns texts into fixed-size embedding vectors.
///
/// Implementations must be deterministic (same input → same output) and
/// thread-safe, and must not access the network. Vectors should be
/// L2-normalized so that [`cosine`] is a plain dot product, but [`cosine`]
/// normalizes defensively anyway.
pub trait Embedder: Send + Sync {
    /// Embed a batch of texts. Returns one vector per input, in input order.
    ///
    /// The e5 "passage: " prefix is applied inside the e5 embedder itself,
    /// never by callers (document mode needs it on both sides).
    fn embed(&self, texts: &[&str]) -> CoreResult<Vec<Vec<f32>>>;

    /// Human-readable embedder id (audit trail, `DiffResult` flag).
    fn name(&self) -> &'static str {
        "fake-hash"
    }

    /// True when backed by a real trained model (drives
    /// `DiffResult::embeddings_used`).
    fn uses_model(&self) -> bool {
        false
    }
}

/// Cosine similarity of two vectors; 0.0 when either is all-zero.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..n {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a.sqrt() * norm_b.sqrt())).clamp(-1.0, 1.0)
}

/// In-place L2 normalization; leaves all-zero vectors untouched.
pub(crate) fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert!((cosine(&[3.0, 0.0], &[-2.0, 0.0]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_handles_uneven_lengths() {
        assert!((cosine(&[1.0, 2.0, 3.0], &[1.0, 2.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fake_embedder_is_deterministic_and_normalized() {
        let e = FakeHashEmbedder::default();
        let v1 = e.embed(&["Hợp đồng lao động", "payment terms"]).unwrap();
        let v2 = e.embed(&["Hợp đồng lao động", "payment terms"]).unwrap();
        assert_eq!(v1, v2);
        for v in &v1 {
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-4, "norm {norm}");
        }
    }

    #[test]
    fn fake_embedder_ranks_similar_texts_higher() {
        let e = FakeHashEmbedder::default();
        let vs = e
            .embed(&[
                "The payment is due within 30 days",
                "The payment is due within 45 days",
                "Completely unrelated topic about bicycles",
            ])
            .unwrap();
        let (a, b, c) = (&vs[0], &vs[1], &vs[2]);
        let same = cosine(a, b);
        let diff = cosine(a, c);
        assert!(same > diff, "similar {same} must exceed unrelated {diff}");
        assert!((cosine(a, a) - 1.0).abs() < 1e-4);
    }
}
