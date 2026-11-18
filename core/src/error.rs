//! Core error type shared across the pipeline.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("extraction failed: {0}")]
    Extraction(String),
    #[error("diff failed: {0}")]
    Diff(String),
    #[error("alignment failed: {0}")]
    Align(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("llm sidecar error: {0}")]
    Llm(String),
    #[error("ocr error: {0}")]
    Ocr(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("report error: {0}")]
    Report(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Cooperative cancellation of a download or streaming-explain job.
    #[error("cancelled by user")]
    Cancelled,
}

pub type CoreResult<T> = Result<T, CoreError>;
