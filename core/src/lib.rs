//! DiffLens core — extraction, diff, alignment, heuristics, LLM sidecar, OCR.
//!
//! All processing is 100% local. The only network egress in the entire
//! codebase lives in [`download`] (user-initiated model/binary downloads).

pub mod align;
pub mod diff;
pub mod embeddings;
pub mod error;
pub mod extract;
pub mod heuristic;
pub mod model;
pub mod pipeline;
pub mod store;

pub mod download;
pub mod llm;
pub mod ocr;
pub mod report;

pub use error::{CoreError, CoreResult};
pub use model::*;
