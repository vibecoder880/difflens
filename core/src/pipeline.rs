//! Full pipeline: extract → align → diff → enrich (heuristics).
//!
//! [`run`] is the single entry point the Tauri shell calls. The embedder is
//! injected so the caller controls fake-vs-real (the app picks the real e5
//! embedder when the `embeddings-ort` feature is on and the model is
//! present, and falls back to the deterministic fake otherwise; that choice
//! is surfaced through [`DiffResult::embeddings_used`]).
//!
//! 100% local: extraction and inference never touch the network.

use crate::align::{AlignmentResult, align};
use crate::diff::{diff_blocks, summarize};
use crate::embeddings::Embedder;
use crate::error::{CoreError, CoreResult};
use crate::extract::extract;
use crate::model::{DiffResult, DocumentModel};
use crate::ocr::OcrOptions;

/// Pipeline stages for progress reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Extract,
    /// Experimental OCR fallback over scanned pages (phase 06; emitted
    /// between extract and align when the OCR path engages).
    Ocr,
    Align,
    Diff,
    Enrich,
}

impl Stage {
    /// Stable id used in `diff://progress` event payloads.
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Extract => "extract",
            Stage::Ocr => "ocr",
            Stage::Align => "align",
            Stage::Diff => "diff",
            Stage::Enrich => "enrich",
        }
    }
}

/// Progress callback: (stage, percent 0..=100). Returns `Err` to abort the
/// pipeline (the shell uses this for cancelation at stage boundaries).
pub type ProgressFn<'a> = dyn FnMut(Stage, u8) -> CoreResult<()> + 'a;

/// Diff two documents with the given embedder.
pub fn run(old_path: &str, new_path: &str, embedder: &dyn Embedder) -> CoreResult<DiffResult> {
    run_with_progress(
        old_path,
        new_path,
        embedder,
        &OcrOptions::default(),
        &mut |_, _| Ok(()),
    )
}

/// [`run`] with progress callbacks (used by the shell's job runner).
///
/// `ocr` controls the experimental OCR fallback (phase 06): default
/// enables *auto* mode, where suspected-scanned PDFs gain OCR blocks only
/// when a user-installed Tesseract with Vietnamese data is detectable.
pub fn run_with_progress(
    old_path: &str,
    new_path: &str,
    embedder: &dyn Embedder,
    ocr: &OcrOptions,
    progress: &mut ProgressFn<'_>,
) -> CoreResult<DiffResult> {
    progress(Stage::Extract, 2)?;
    let mut old_doc: DocumentModel = extract(old_path)?;
    let mut new_doc: DocumentModel = extract(new_path)?;
    if old_doc.format != new_doc.format {
        return Err(CoreError::Diff(format!(
            "format mismatch: {} is {:?} but {} is {:?}",
            old_path, old_doc.format, new_path, new_doc.format
        )));
    }
    progress(Stage::Extract, 15)?;

    // Experimental OCR fallback over textless PDF pages (phase 06). Never
    // fails the pipeline; progress maps the OCR callback into 16..=26%.
    crate::ocr::augment_with_ocr(&mut old_doc, old_path, ocr, &mut |p| {
        progress(Stage::Ocr, 16 + ((p.min(100) as u16) * 10 / 100) as u8)
    })?;
    crate::ocr::augment_with_ocr(&mut new_doc, new_path, ocr, &mut |p| {
        progress(Stage::Ocr, 16 + ((p.min(100) as u16) * 10 / 100) as u8)
    })?;

    progress(Stage::Align, 30)?;
    let alignment: AlignmentResult = align(&old_doc.blocks, &new_doc.blocks, embedder)?;
    progress(Stage::Align, 70)?;

    progress(Stage::Diff, 75)?;
    let pairs: Vec<(usize, usize)> = alignment.pairs.iter().map(|p| (p.old, p.new)).collect();
    let mut changes = diff_blocks(&old_doc.blocks, &new_doc.blocks, &pairs);
    let stats = summarize(&changes);
    progress(Stage::Diff, 85)?;

    progress(Stage::Enrich, 90)?;
    let audit = crate::heuristic::enrich_changes(&mut changes, &old_doc.blocks, &new_doc.blocks);
    progress(Stage::Enrich, 100)?;

    Ok(DiffResult {
        old_path: old_doc.path,
        new_path: new_doc.path,
        format: old_doc.format,
        changes,
        stats,
        embeddings_used: embedder.uses_model(),
        audit,
    })
}

/// Pick the best available embedder for this build: real e5-small under the
/// `embeddings-ort` feature when the model files are present, deterministic
/// fake otherwise. Never fails — falls back with a logged warning.
pub fn default_embedder() -> Box<dyn Embedder> {
    #[cfg(feature = "embeddings-ort")]
    {
        match crate::embeddings::ort_e5::OrtE5Embedder::from_default_dir() {
            Ok(e) => {
                tracing::info!("using real e5-small embeddings");
                Box::new(e)
            }
            Err(e) => {
                tracing::warn!(
                    "real embeddings unavailable ({e}); falling back to the fake embedder"
                );
                Box::new(crate::embeddings::FakeHashEmbedder::default())
            }
        }
    }
    #[cfg(not(feature = "embeddings-ort"))]
    {
        Box::new(crate::embeddings::FakeHashEmbedder::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::FakeHashEmbedder;
    use std::io::Write as _;

    fn write_doc(dir: &std::path::Path, name: &str, content: &str) -> String {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn round_trip_over_txt_fixtures() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_doc(dir.path(), "a.txt", "# Contract\n\nPayment in 30 days.\n");
        let b = write_doc(
            dir.path(),
            "b.txt",
            "# Contract\n\nPayment in 45 days.\n\nExtra clause.\n",
        );
        let result = run(&a, &b, &FakeHashEmbedder::default()).unwrap();
        assert_eq!(result.format, crate::model::DocFormat::Txt);
        assert!(!result.embeddings_used);
        assert_eq!(result.stats.modified, 1);
        assert_eq!(result.stats.added, 1);
        assert!(result.changes[0].id == "c0");
        // Added changes reach the shell payload with their block text.
        let added = result
            .changes
            .iter()
            .find(|c| c.kind == crate::model::ChangeKind::Added)
            .expect("added change present");
        assert_eq!(added.new_text.as_deref(), Some("Extra clause."));
        assert!(added.old_text.is_none());
    }

    #[test]
    fn identical_files_are_all_equal() {
        let dir = tempfile::tempdir().unwrap();
        let text = "# Report\n\nAlpha paragraph.\n\nBeta paragraph.\n";
        let a = write_doc(dir.path(), "same_a.txt", text);
        let b = write_doc(dir.path(), "same_b.txt", text);
        let result = run(&a, &b, &FakeHashEmbedder::default()).unwrap();
        assert_eq!(result.stats.equal, 3);
        assert_eq!(
            result
                .changes
                .iter()
                .filter(|c| c.kind == crate::model::ChangeKind::Equal)
                .count(),
            3
        );
    }

    #[test]
    fn format_mismatch_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_doc(dir.path(), "x.txt", "plain\n");
        let docx =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.docx");
        let err = run(&a, docx.to_str().unwrap(), &FakeHashEmbedder::default()).unwrap_err();
        assert!(err.to_string().contains("format mismatch"));
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let err = run(
            "/nonexistent/a.txt",
            "/nonexistent/b.txt",
            &FakeHashEmbedder::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("cannot read"));
    }

    #[test]
    fn determinism_same_inputs_same_output() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_doc(
            dir.path(),
            "d_a.txt",
            "# T\n\nAlpha.\n\nBeta body.\n\nGamma ends.\n",
        );
        let b = write_doc(
            dir.path(),
            "d_b.txt",
            "# T\n\nAlpha changed.\n\nBeta body.\n\nGamma ends.\n",
        );
        let e = FakeHashEmbedder::default();
        let r1 = serde_json::to_string(&run(&a, &b, &e).unwrap()).unwrap();
        let r2 = serde_json::to_string(&run(&a, &b, &e).unwrap()).unwrap();
        assert_eq!(r1, r2);
    }
}
