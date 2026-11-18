//! OCR integration tests (phase 06, experimental Tesseract path).
//!
//! Unit-level coverage (TSV parsing, geometry reconstruction, TIFF encoding,
//! detection stubbing, subprocess timeouts) lives inside `core/src/ocr/*`.
//! The end-to-end tests here need a real `tesseract` CLI with the Vietnamese
//! traineddata (`vie`), so they are `#[ignore]`d:
//!
//! - locally (no tesseract installed) they skip with a clear message;
//! - CI's core job installs `tesseract-ocr tesseract-ocr-vie`, sets
//!   `DIFFLENS_OCR_STRICT=1` (missing OCR then fails the job, like
//!   `DIFFLENS_PDFIUM_STRICT`) and runs them with `-- --ignored ocr_`.
//!
//! Like `extract_tests.rs`, a missing PDFium library skips when the strict
//! env is unset (both fixtures are PDF-based).

use std::path::PathBuf;

use difflens_core::embeddings::FakeHashEmbedder;
use difflens_core::extract::normalize::normalize;
use difflens_core::ocr::{OcrOptions, detect, run_tesseract_image};
use difflens_core::pipeline;

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
        .to_str()
        .unwrap()
        .to_string()
}

/// Skip (or fail under `DIFFLENS_OCR_STRICT`) when tesseract + `vie` are
/// missing. Returns the availability when the e2e assertions may run.
fn ocr_or_skip() -> Option<difflens_core::ocr::OcrAvailability> {
    let avail = detect();
    if avail.usable() {
        return Some(avail);
    }
    if std::env::var("DIFFLENS_OCR_STRICT").is_ok() {
        panic!(
            "DIFFLENS_OCR_STRICT is set but tesseract with the 'vie' language data is missing \
             (install tesseract-ocr + tesseract-ocr-vie)"
        );
    }
    eprintln!(
        "skipping OCR e2e assertions: tesseract with 'vie' not found on PATH \
         (install tesseract-ocr and tesseract-ocr-vie to run them)"
    );
    None
}

/// Skip (or fail) when PDFium is not staged — the scanned-PDF fixture needs
/// it for rasterization. Mirrors `extract_tests::pdf_strict`.
fn extract_pdf_or_skip(name: &str) -> Option<difflens_core::DocumentModel> {
    match difflens_core::extract::extract(&fixture(name)) {
        Ok(model) => Some(model),
        Err(e) => {
            let msg = e.to_string();
            let library_missing =
                msg.contains("PDFium library not found") || msg.contains("could not be bound");
            if !library_missing {
                panic!("pdf extraction failed unexpectedly: {e}");
            }
            if std::env::var("DIFFLENS_PDFIUM_STRICT").is_ok() {
                panic!("DIFFLENS_PDFIUM_STRICT is set but PDFium is missing: {e}");
            }
            eprintln!("skipping OCR pdf assertions ({name}): {msg}");
            None
        }
    }
}

/// Vietnamese diacritics count — diacritic survival is the core OCR quality
/// signal asserted by CI (the exact character set covers all marks used in
/// the fixture text).
fn count_vietnamese_diacritics(text: &str) -> usize {
    text.chars()
        .filter(|c| {
            "ăâêôơưđĐÀÁẢÃẠĂÂẦẨẪẬẮẰẲẴẶÊỀỂỄỆÓÒỎÕỌÔỐỒỖỘƠỚỜỞỠỢÚÙỦŨỤƯỨỪỬỮỰÝỲỶỸỴáàảãạăâấầẩẫậắằẳẵặ\
             éèẻẽẹếềểễệíìỉĩịóòỏõọốồổỗộớờởỡợúùủũụứừửữựýỳỷỹỵ"
                .contains(*c)
        })
        .count()
}

// --------------------------------------------------------------------------
// Engine-level: tesseract directly on the committed PNG fixture.
// --------------------------------------------------------------------------

#[test]
#[ignore]
fn ocr_png_engine_recovers_vietnamese_contract_words() {
    let Some(avail) = ocr_or_skip() else { return };
    let words = run_tesseract_image(
        avail.binary.as_ref().unwrap(),
        &avail.lang_arg(),
        std::path::Path::new(&fixture("vietnamese_scanned.png")),
        std::time::Duration::from_secs(120),
    )
    .expect("tesseract runs over the committed PNG fixture");

    assert!(!words.is_empty(), "OCR produced no words");
    let text = normalize(
        &words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let mean_conf = words.iter().map(|w| w.conf).sum::<f32>() / words.len() as f32;

    // Contract vocabulary must survive OCR verbatim (asserted on clean
    // rendered input with tesseract 5 + vie; see phase-06 plan for the
    // honest ~18% CER caveat on real-world scans).
    // Case-insensitive: sentence-position words are lowercase in the fixture.
    let lowered = text.to_lowercase();
    assert!(
        lowered.contains("thanh toán"),
        "missing 'thanh toán' in: {text}"
    );
    assert!(
        lowered.contains("hợp đồng"),
        "missing 'hợp đồng' in: {text}"
    );
    assert!(
        count_vietnamese_diacritics(&text) >= 10,
        "diacritics lost in OCR output: {text}"
    );
    assert!(
        mean_conf > 60.0,
        "mean confidence {mean_conf} too low for clean rendered text"
    );

    println!("ocr-ci-ok: conf={mean_conf:.1} text={text}");
}

// --------------------------------------------------------------------------
// Document-level: extract → OCR fallback merge (honest provenance).
// --------------------------------------------------------------------------

#[test]
#[ignore]
fn ocr_scanned_pdf_blocks_carry_experimental_provenance() {
    let Some(_) = ocr_or_skip() else { return };
    let Some(model) = extract_pdf_or_skip("vietnamese_scanned.pdf") else {
        return;
    };
    // The fixture is image-only: the extractor must flag it as scanned.
    assert!(
        model
            .warnings
            .iter()
            .any(|w| w.contains("scanned pdf suspected")),
        "image-only fixture must trip the scanned-pdf warning: {:?}",
        model.warnings
    );
    assert!(model.blocks.is_empty(), "image-only PDF has no text layer");

    let mut doc = model;
    difflens_core::ocr::augment_with_ocr(
        &mut doc,
        &fixture("vietnamese_scanned.pdf"),
        &OcrOptions::default(),
        &mut |_| Ok(()),
    )
    .expect("OCR fallback never fails the document");

    let ocr_blocks: Vec<_> = doc
        .blocks
        .iter()
        .filter(|b| b.anchor.as_deref() == Some("ocr"))
        .collect();
    assert!(!ocr_blocks.is_empty(), "OCR produced no provenance blocks");
    assert!(
        ocr_blocks
            .iter()
            .all(|b| b.page == Some(1) && b.rect.is_some()),
        "OCR blocks must carry page + rect"
    );
    assert!(
        doc.warnings
            .iter()
            .any(|w| w.contains("extracted via OCR (experimental)")),
        "missing honest OCR provenance warning: {:?}",
        doc.warnings
    );
    assert!(
        doc.warnings.iter().any(|w| w.contains("mean confidence")),
        "missing confidence metadata warning: {:?}",
        doc.warnings
    );

    let text = doc
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let lowered = text.to_lowercase();
    assert!(
        lowered.contains("thanh toán"),
        "missing 'thanh toán' in: {text}"
    );
    assert!(
        lowered.contains("hợp đồng"),
        "missing 'hợp đồng' in: {text}"
    );
    assert!(
        count_vietnamese_diacritics(&text) >= 10,
        "diacritics lost in OCR output: {text}"
    );
    println!(
        "ocr-ci-ok: {} OCR blocks, warnings={:?}",
        doc.blocks.len(),
        doc.warnings
    );
}

// --------------------------------------------------------------------------
// Pipeline-level: OCR blocks flow through the normal diff path unchanged.
// --------------------------------------------------------------------------

#[test]
#[ignore]
fn ocr_pipeline_identical_scanned_inputs_are_all_equal() {
    let Some(_) = ocr_or_skip() else { return };
    let path = fixture("vietnamese_scanned.pdf");

    // Also requires PDFium (the scanned fixture path extracts + rasterizes).
    let probe = difflens_core::extract::extract(&path);
    if let Err(e) = probe {
        let msg = e.to_string();
        if msg.contains("PDFium library not found") || msg.contains("could not be bound") {
            if std::env::var("DIFFLENS_PDFIUM_STRICT").is_ok() {
                panic!("DIFFLENS_PDFIUM_STRICT is set but PDFium is missing: {e}");
            }
            eprintln!("skipping OCR pipeline assertions: {msg}");
            return;
        }
        panic!("pdf extraction failed unexpectedly: {e}");
    }

    let mut stages: Vec<String> = Vec::new();
    let result = pipeline::run_with_progress(
        &path,
        &path,
        &FakeHashEmbedder::default(),
        &OcrOptions::default(),
        &mut |stage, percent| {
            stages.push(format!("{}:{percent}", stage.as_str()));
            Ok(())
        },
    )
    .expect("pipeline runs over the scanned fixture");

    // Identical inputs → every change Equal, driven by OCR-sourced blocks.
    assert!(
        result
            .changes
            .iter()
            .all(|c| c.kind == difflens_core::ChangeKind::Equal),
        "identical scanned inputs must diff as Equal"
    );
    assert!(
        result.stats.equal >= 3,
        "OCR blocks must reach the diff stage"
    );

    // The OCR stage was announced between extract and align.
    let ocr_pos = stages.iter().position(|s| s.starts_with("ocr:"));
    let align_pos = stages.iter().position(|s| s.starts_with("align:"));
    assert!(
        ocr_pos.is_some() && Some(align_pos.unwrap_or(999)) > ocr_pos,
        "ocr stage must be emitted before align: {stages:?}"
    );
    println!("ocr-ci-ok: pipeline stages={stages:?}");
}
