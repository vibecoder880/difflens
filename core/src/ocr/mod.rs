//! Experimental OCR path for scanned PDFs (phase 06, Tesseract CLI).
//!
//! Tesseract is **detected, not shipped**: when the user has installed the
//! `tesseract` CLI with Vietnamese traineddata (`vie`), textless PDF pages
//! (the Phase-02 "scanned pdf suspected" case) are rasterized at 300 DPI
//! via the Phase-02 pdfium loader, OCR'd page by page under a hard
//! per-page timeout, and merged back into the normal [`DocumentModel`] as
//! ordinary blocks — downstream diff/alignment/heuristics work unchanged.
//!
//! Honest labeling is a contract: OCR blocks carry `anchor: Some("ocr")`,
//! the document gains a "pages … extracted via OCR (experimental)"
//! warning, and every user-facing string lives in [`labels`]. When
//! Tesseract or `vie` is missing the feature silently stays off (one
//! warning); extraction **never fails** because of OCR.
//!
//! 100% local: the only subprocess is the user's own tesseract binary,
//! invoked with fixed argument shapes on temp-dir files.

pub mod detect;
pub mod engine;
pub mod labels;
pub mod proc;
pub mod rasterize;

use std::time::Duration;

use crate::error::CoreResult;
use crate::model::{DocFormat, DocumentModel};

pub use detect::{OcrAvailability, detect, detect_with};
pub use engine::{OcrOutcome, ocr_pdf, parse_tsv, run_tesseract_image, words_to_blocks};
pub use labels::{OCR_LABEL_KEYS, label_for_warning};

/// The Phase-02 scanned-PDF threshold, reused for OCR fallback gating.
use crate::extract::pdf::SCANNED_PAGE_RATIO;

/// Options for the experimental OCR path (pipeline parameter).
///
/// Default = **enabled-auto**: the pipeline runs OCR only when the document
/// is suspected-scanned (textless-page ratio above
/// [`SCANNED_PAGE_RATIO`]) *and* Tesseract with `vie` is detectable.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrOptions {
    /// `false` disables the OCR path entirely (no detection, no warning).
    pub enabled: bool,
    /// Rasterization DPI (300 = OCR-friendly target, per phase contract).
    pub dpi: f32,
    /// Maximum pages OCR'd per job (phase cap: 50; beyond → warning + stop).
    pub max_pages: u32,
    /// Hard per-page tesseract timeout.
    pub page_timeout: Duration,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            dpi: 300.0,
            max_pages: 50,
            page_timeout: Duration::from_secs(90),
        }
    }
}

/// Detect OCR availability for the current machine (thin alias kept for
/// call-site readability in the shell's `ocr_status` command).
pub fn availability() -> OcrAvailability {
    detect()
}

/// Append OCR blocks to `doc` for its textless pages, when the OCR path is
/// enabled, the document is a suspected-scanned PDF and Tesseract+`vie` is
/// available. Never fails and never panics; all degradation is via
/// `doc.warnings` (honest labels from [`labels`]).
///
/// `progress` receives OCR-stage progress 0..=100 (mapped to the pipeline
/// `ocr` stage by the caller).
pub fn augment_with_ocr(
    doc: &mut DocumentModel,
    path: &str,
    opts: &OcrOptions,
    progress: &mut dyn FnMut(u8) -> CoreResult<()>,
) -> CoreResult<()> {
    if !opts.enabled || doc.format != DocFormat::Pdf {
        return Ok(());
    }

    // Sparse-text-layer detection: same criterion and threshold as the
    // Phase-2 "scanned pdf suspected" warning (pages with an empty text
    // layer, ratio > 30%).
    let (textless, total) = match crate::extract::pdf::scan_textless_pages(path) {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!("OCR fallback: could not re-open {path}: {e}");
            return Ok(());
        }
    };
    if total == 0 || textless.is_empty() {
        return Ok(());
    }
    let ratio = textless.len() as f32 / total as f32;
    if ratio <= SCANNED_PAGE_RATIO {
        // Not scanned enough to justify the experimental path.
        return Ok(());
    }

    let availability = detect();
    if !availability.usable() {
        doc.warnings.push(labels::unavailable());
        return Ok(());
    }

    // Page cap (phase contract: 50 per job).
    let mut capped = textless.clone();
    if capped.len() > opts.max_pages as usize {
        capped.truncate(opts.max_pages as usize);
        doc.warnings
            .push(labels::page_cap(opts.max_pages, textless.len()));
    }

    progress(2)?;
    let outcome = engine::ocr_pdf(path, &capped, &availability, opts.dpi, opts.page_timeout)?;
    let pages_total = capped.len();
    for (done, _) in outcome.pages_done.iter().enumerate() {
        // Spread 3..=97 across the pages actually processed.
        let percent = 3 + (((done + 1) as f32 / pages_total.max(1) as f32) * 94.0).round() as u8;
        progress(percent.min(97))?;
    }

    if outcome.pages_done.is_empty() {
        // Every page failed; keep the honest warnings, add no blocks.
        doc.warnings.extend(outcome.warnings);
        return Ok(());
    }

    doc.warnings
        .push(labels::pages_via_ocr(&outcome.pages_done));
    doc.warnings.push(labels::mean_confidence(
        outcome.mean_conf,
        outcome.pages_done.len(),
    ));
    doc.warnings.extend(outcome.warnings);
    doc.blocks.extend(outcome.blocks);
    progress(100)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, BlockKind};

    fn pdf_doc() -> DocumentModel {
        DocumentModel {
            path: "x.pdf".into(),
            format: DocFormat::Pdf,
            blocks: Vec::new(),
            page_count: Some(1),
            warnings: Vec::new(),
        }
    }

    fn no_progress(_: u8) -> CoreResult<()> {
        Ok(())
    }

    #[test]
    fn default_options_are_enabled_auto() {
        let opts = OcrOptions::default();
        assert!(opts.enabled);
        assert_eq!(opts.dpi, 300.0);
        assert_eq!(opts.max_pages, 50);
        assert_eq!(opts.page_timeout, Duration::from_secs(90));
    }

    #[test]
    fn disabled_option_never_touches_the_doc() {
        let mut doc = pdf_doc();
        augment_with_ocr(
            &mut doc,
            "/nonexistent.pdf",
            &OcrOptions {
                enabled: false,
                ..Default::default()
            },
            &mut no_progress,
        )
        .unwrap();
        assert!(doc.warnings.is_empty());
        assert!(doc.blocks.is_empty());
    }

    #[test]
    fn non_pdf_formats_are_skipped() {
        let mut doc = pdf_doc();
        doc.format = DocFormat::Txt;
        augment_with_ocr(
            &mut doc,
            "/nonexistent.txt",
            &OcrOptions::default(),
            &mut no_progress,
        )
        .unwrap();
        assert!(doc.warnings.is_empty());
    }

    #[test]
    fn missing_file_degrades_without_failure() {
        let mut doc = pdf_doc();
        augment_with_ocr(
            &mut doc,
            "/nonexistent/x.pdf",
            &OcrOptions::default(),
            &mut no_progress,
        )
        .unwrap();
        assert!(doc.warnings.is_empty());
        assert!(doc.blocks.is_empty());
    }

    #[test]
    fn ocr_blocks_carry_anchor_and_page_provenance() {
        // Direct block-level provenance check (no tesseract needed).
        let words = vec![super::engine::OcrWord {
            left: 10,
            top: 10,
            width: 100,
            height: 20,
            conf: 90.0,
            text: "scanned".into(),
        }];
        let blocks: Vec<Block> = words_to_blocks(&words, 4, 200.0, 300.0);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].anchor.as_deref(), Some("ocr"));
        assert_eq!(blocks[0].page, Some(4));
        assert_eq!(blocks[0].kind, BlockKind::Paragraph);
    }
}
