//! Real embedding inference through ONNX Runtime (`ort`), behind the
//! `embeddings-ort` feature.
//!
//! Loads the pinned multilingual-e5-small ONNX export (`Xenova/
//! multilingual-e5-small`, see `scripts/fetch-models.sh` for commit + SHA256)
//! plus its `tokenizer.json`, applies the e5 `passage: ` prefix on every
//! input (document mode: both sides of a diff get the same prefix), mean
//! pools the last hidden state with the attention mask and L2-normalizes to
//! 384-dim vectors.
//!
//! 100% local: inference reads only the configured model dir; the network is
//! never touched here (fetching is the script's job).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ort::environment::Environment;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use tokenizers::{Tokenizer, TruncationParams};

use super::{Embedder, l2_normalize};
use crate::error::{CoreError, CoreResult};

/// e5 protocol prefix, applied to every text (document mode: both sides).
const PASSAGE_PREFIX: &str = "passage: ";
/// Default ONNX intra-op thread count (the dev/CI VM has 3 cores).
pub const DEFAULT_THREADS: usize = 3;
/// Default token cap per input (e5-small context window).
pub const DEFAULT_MAX_TOKENS: usize = 512;
/// Default texts per batch — small so 6 GB RAM machines stay comfortable.
pub const DEFAULT_BATCH_SIZE: usize = 8;
/// Dimension of e5-small embeddings.
pub const E5_DIM: usize = 384;

/// ONNX Runtime e5 embedder.
pub struct OrtE5Embedder {
    env: ort::environment::Environment,
    /// `run` takes `&mut self`, so inference is serialized through a mutex.
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    pad_id: u32,
    batch_size: usize,
}

impl OrtE5Embedder {
    /// Load model + tokenizer from a directory laid out as
    /// `onnx/model_quantized.onnx` + `tokenizer.json`.
    pub fn from_dir(dir: &Path) -> CoreResult<Self> {
        Self::from_dir_with(dir, DEFAULT_THREADS, DEFAULT_MAX_TOKENS, DEFAULT_BATCH_SIZE)
    }

    /// Same as [`from_dir`](Self::from_dir) with explicit tuning knobs.
    pub fn from_dir_with(
        dir: &Path,
        threads: usize,
        max_tokens: usize,
        batch_size: usize,
    ) -> CoreResult<Self> {
        let model_path = dir.join("onnx").join("model_quantized.onnx");
        let tokenizer_path = dir.join("tokenizer.json");
        for p in [&model_path, &tokenizer_path] {
            if !p.is_file() {
                return Err(CoreError::Align(format!(
                    "e5 model files missing: {} not found. Run `scripts/fetch-models.sh --test` \
                     or point DIFFLENS_E5_DIR at a directory containing \
                     onnx/model_quantized.onnx and tokenizer.json",
                    p.display()
                )));
            }
        }

        let env = ort::init()
            .build()
            .map_err(|e| CoreError::Align(format!("onnxruntime init failed: {e}")))?;

        let session = (|| {
            let mut builder = Session::builder(&env)?
                .with_optimization_level(GraphOptimizationLevel::Level3)?
                .with_intra_threads(threads)?;
            builder.commit_from_file(&model_path)
        })()
        .map_err(|e| CoreError::Align(format!("cannot load {}: {e}", model_path.display())))?;

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            CoreError::Align(format!("cannot load {}: {e}", tokenizer_path.display()))
        })?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                direction: Default::default(),
                max_length: max_tokens,
                strategy: Default::default(),
                stride: 0,
            }))
            .map_err(|e| CoreError::Align(format!("tokenizer truncation setup failed: {e}")))?;

        let pad_id = tokenizer
            .token_to_id("<pad>")
            .ok_or_else(|| CoreError::Align("tokenizer has no <pad> token".to_string()))?;

        Ok(Self {
            env,
            session: Mutex::new(session),
            tokenizer,
            pad_id,
            batch_size,
        })
    }

    /// Resolve the model directory: `DIFFLENS_E5_DIR` if set, else the repo
    /// `resources/models/e5-small` layout produced by `fetch-models.sh`.
    pub fn default_dir() -> CoreResult<PathBuf> {
        if let Ok(dir) = std::env::var("DIFFLENS_E5_DIR") {
            return Ok(PathBuf::from(dir));
        }
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let candidates = [
            manifest_dir.join("../resources/models/e5-small"),
            PathBuf::from("resources/models/e5-small"),
        ];
        for c in &candidates {
            if c.join("onnx").join("model_quantized.onnx").is_file() {
                return Ok(c.clone());
            }
        }
        Err(CoreError::Align(format!(
            "e5 model directory not found; tried DIFFLENS_E5_DIR (unset) and {}. \
             Run `scripts/fetch-models.sh --test` first",
            candidates
                .iter()
                .map(|c| c.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }

    /// Embedder backed by the default model location.
    pub fn from_default_dir() -> CoreResult<Self> {
        Self::from_dir(&Self::default_dir()?)
    }

    /// The shared ONNX Runtime environment this session was built from.
    pub fn environment(&self) -> &Environment {
        &self.env
    }

    /// Mean-pool one row of the last hidden state with its attention mask.
    fn mean_pool(hidden: &[f32], hidden_dim: usize, mask: &[u32], token_count: usize) -> Vec<f32> {
        let mut pooled = vec![0.0f32; hidden_dim];
        let mut denom = 0.0f32;
        for t in 0..token_count.min(mask.len()) {
            let weight = f32::from(mask[t] != 0);
            denom += weight;
            for h in 0..hidden_dim {
                pooled[h] += hidden[t * hidden_dim + h] * weight;
            }
        }
        if denom > 0.0 {
            for x in pooled.iter_mut() {
                *x /= denom;
            }
        }
        l2_normalize(&mut pooled);
        pooled
    }

    /// Run one padded batch through the session and return pooled vectors
    /// in batch order.
    fn run_batch(&self, batch: &[&tokenizers::Encoding]) -> CoreResult<Vec<Vec<f32>>> {
        let seq_len = batch
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(1)
            .max(1);
        let n = batch.len();

        let mut ids = vec![i64::from(self.pad_id); n * seq_len];
        let mut mask = vec![0u32; n * seq_len];
        for (row, enc) in batch.iter().enumerate() {
            let toks = enc.get_ids();
            let att = enc.get_attention_mask();
            for (col, (t, m)) in toks.iter().zip(att.iter()).enumerate() {
                ids[row * seq_len + col] = i64::from(*t);
                mask[row * seq_len + col] = *m;
            }
        }
        let mask_i64: Vec<i64> = mask.iter().map(|m| i64::from(*m)).collect();

        let mut session = self
            .session
            .lock()
            .map_err(|_| CoreError::Align("e5 session lock poisoned".to_string()))?;

        let has_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        let shape_vec = vec![n as i64, seq_len as i64];

        let mut inputs: Vec<(&str, SessionInputValue<'_>)> = Vec::with_capacity(3);
        inputs.push((
            "input_ids",
            Tensor::from_array((shape_vec.clone(), ids.into_boxed_slice()))
                .map_err(|e| CoreError::Align(format!("input_ids tensor: {e}")))?
                .into(),
        ));
        inputs.push((
            "attention_mask",
            Tensor::from_array((shape_vec.clone(), mask_i64.into_boxed_slice()))
                .map_err(|e| CoreError::Align(format!("attention_mask tensor: {e}")))?
                .into(),
        ));
        if has_token_type_ids {
            inputs.push((
                "token_type_ids",
                Tensor::from_array((shape_vec, vec![0i64; n * seq_len].into_boxed_slice()))
                    .map_err(|e| CoreError::Align(format!("token_type_ids tensor: {e}")))?
                    .into(),
            ));
        }

        let outputs = session
            .run(inputs)
            .map_err(|e| CoreError::Align(format!("e5 inference failed: {e}")))?;

        // The pinned Xenova export names its output `last_hidden_state`;
        // anything else is a loud configuration error, not a guess.
        let value = outputs.get("last_hidden_state").ok_or_else(|| {
            let available = outputs
                .iter()
                .map(|(name, _)| name.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            CoreError::Align(format!(
                "e5 model has no last_hidden_state output (has: {available})"
            ))
        })?;
        let (shape, hidden) = value
            .try_extract_tensor::<f32>()
            .map_err(|e| CoreError::Align(format!("e5 output not f32 tensor: {e}")))?;
        if shape.len() != 3 {
            return Err(CoreError::Align(format!(
                "unexpected e5 output rank {} (want [batch, seq, hidden])",
                shape.len()
            )));
        }
        let hidden_dim = shape[2] as usize;
        if hidden_dim == 0 || hidden.len() < n * seq_len * hidden_dim {
            return Err(CoreError::Align(
                "e5 output tensor smaller than expected".to_string(),
            ));
        }

        let mut out = Vec::with_capacity(n);
        for row in 0..n {
            let start = row * seq_len * hidden_dim;
            let token_count = batch[row].get_ids().len();
            out.push(Self::mean_pool(
                &hidden[start..start + token_count * hidden_dim],
                hidden_dim,
                &mask[row * seq_len..row * seq_len + seq_len],
                token_count,
            ));
        }
        Ok(out)
    }
}

impl Embedder for OrtE5Embedder {
    fn embed(&self, texts: &[&str]) -> CoreResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Tokenize everything up-front, then batch length-sorted to minimize
        // padding waste on the 3-core budget.
        let mut ordered: Vec<(usize, tokenizers::Encoding)> = Vec::with_capacity(texts.len());
        for (i, text) in texts.iter().enumerate() {
            let prefixed = format!("{PASSAGE_PREFIX}{text}");
            let enc = self
                .tokenizer
                .encode(prefixed, true)
                .map_err(|e| CoreError::Align(format!("tokenization failed for text #{i}: {e}")))?;
            ordered.push((i, enc));
        }
        ordered.sort_by_key(|(_, e)| e.get_ids().len());

        let mut results: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        for chunk in ordered.chunks(self.batch_size) {
            let refs: Vec<&tokenizers::Encoding> = chunk.iter().map(|(_, e)| e).collect();
            let pooled = self.run_batch(&refs)?;
            for ((orig, _), v) in chunk.iter().zip(pooled) {
                results[*orig] = Some(v);
            }
        }

        results
            .into_iter()
            .enumerate()
            .map(|(i, v)| v.ok_or_else(|| CoreError::Align(format!("embedding row {i} missing"))))
            .collect()
    }

    fn name(&self) -> &'static str {
        "ort-e5-small"
    }

    fn uses_model(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration checks live in `core/tests/gold_tests.rs`
    /// (`embeddings_smoke_real` / `embeddings_gold_alignment_real`), gated on
    /// the fetched model. Here we only test path resolution errors.
    #[test]
    fn missing_files_fail_loudly() {
        let tmp = tempfile::tempdir().unwrap();
        let err = match OrtE5Embedder::from_dir(tmp.path()) {
            Err(e) => e,
            Ok(_) => panic!("expected a missing-model error"),
        };
        assert!(err.to_string().contains("model files missing"));
    }

    #[test]
    fn default_dir_mentions_fetch_script_when_missing() {
        // Only meaningful when the model really is absent; when present the
        // resolution succeeds and the error path is unreachable.
        if std::env::var("DIFFLENS_E5_DIR").is_ok() {
            return;
        }
        if OrtE5Embedder::default_dir().is_ok() {
            return;
        }
        let err = OrtE5Embedder::default_dir().unwrap_err();
        assert!(err.to_string().contains("fetch-models.sh"));
    }

    /// Tokenizer wiring check — pure Rust (no ONNX Runtime involvement), so
    /// it runs on CPUs without AVX2 too. Requires the fetched model dir.
    #[test]
    fn tokenizer_layout_and_e5_prefix() {
        let dir = match OrtE5Embedder::default_dir() {
            Ok(d) => d,
            Err(_) => return, // model not fetched; the ignored tests cover it
        };
        let tokenizer_path = dir.join("tokenizer.json");
        let mut tokenizer =
            Tokenizer::from_file(&tokenizer_path).expect("fetched tokenizer.json must load");
        assert!(
            tokenizer.token_to_id("<pad>").is_some(),
            "pad token present"
        );
        assert!(tokenizer.token_to_id("</s>").is_some(), "eos token present");

        tokenizer
            .with_truncation(Some(TruncationParams {
                direction: Default::default(),
                max_length: DEFAULT_MAX_TOKENS,
                strategy: Default::default(),
                stride: 0,
            }))
            .expect("truncation applies");

        let enc = tokenizer
            .encode(
                format!("{PASSAGE_PREFIX}Thanh toán trong vòng 30 ngày"),
                true,
            )
            .expect("vietnamese text encodes");
        let ids = enc.get_ids();
        let mask = enc.get_attention_mask();
        assert!(!ids.is_empty());
        assert_eq!(ids.len(), mask.len());
        assert_eq!(mask[0], 1);
        assert!(ids.iter().all(|id| (*id as usize) < 300_000));
        // Special tokens wrap the sequence.
        let bos = tokenizer.token_to_id("<s>").unwrap();
        let eos = tokenizer.token_to_id("</s>").unwrap();
        assert_eq!(ids[0], bos);
        assert_eq!(*ids.last().unwrap(), eos);
    }

    #[test]
    fn knobs_are_sane() {
        assert_eq!(E5_DIM, 384);
        assert!((8..=16).contains(&DEFAULT_BATCH_SIZE));
        assert_eq!(DEFAULT_THREADS, 3);
        assert_eq!(PASSAGE_PREFIX, "passage: ");
    }
}
