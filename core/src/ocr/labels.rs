//! User-facing OCR warning strings and their i18n label keys.
//!
//! Mirrors the established `heuristic/labels.rs` pattern: every user-facing
//! string this module can emit into `DocumentModel.warnings` is built here,
//! and every string maps to a stable label key listed in [`OCR_LABEL_KEYS`]
//! (Phase 7's frontend translation files validate against that list —
//! emitting an unlisted key breaks the contract). Keys follow the
//! `ocr.<camelCase>` convention.
//!
//! All strings are honest-labeling by contract: OCR output is experimental
//! and every warning that mentions OCR carries the word "experimental".

/// All label keys the OCR layer may emit, sorted alphabetically
/// (deterministic order for snapshotting).
pub const OCR_LABEL_KEYS: &[&str] = &[
    "ocr.confidence",
    "ocr.experimental",
    "ocr.pageCap",
    "ocr.pageFailed",
    "ocr.unavailable",
    "ocr.unknown",
];

/// Map an emitted warning string to its i18n label key (single source of
/// truth). Matching is structural (prefix/suffix) because warnings embed
/// page numbers and measurements.
pub fn label_for_warning(warning: &str) -> &'static str {
    let w = warning.trim_start();
    if w.starts_with("pages ") && w.ends_with("extracted via OCR (experimental)") {
        return "ocr.experimental";
    }
    if w.starts_with("OCR (experimental) mean confidence") {
        return "ocr.confidence";
    }
    if w.starts_with("OCR (experimental) page cap") {
        return "ocr.pageCap";
    }
    if w.starts_with("OCR (experimental) failed on page") {
        return "ocr.pageFailed";
    }
    if w.starts_with("OCR unavailable") {
        return "ocr.unavailable";
    }
    "ocr.unknown"
}

// ---------------------------------------------------------------------------
// Warning builders — the only OCR warning strings the pipeline emits.
// ---------------------------------------------------------------------------

/// "pages 3,5 extracted via OCR (experimental)" — honest provenance for the
/// pages whose blocks came from OCR rather than a text layer.
pub(crate) fn pages_via_ocr(pages: &[u32]) -> String {
    let list: Vec<String> = pages.iter().map(|p| p.to_string()).collect();
    format!("pages {} extracted via OCR (experimental)", list.join(","))
}

/// Aggregate confidence metadata for a batch of OCR pages (honest quality
/// signal for the UI; the per-word values stay in the Tesseract TSV).
pub(crate) fn mean_confidence(conf: f32, pages: usize) -> String {
    format!("OCR (experimental) mean confidence {conf:.1}% across {pages} page(s)")
}

/// Emitted when more sparse pages exist than the job may process.
pub(crate) fn page_cap(cap: u32, total: usize) -> String {
    format!(
        "OCR (experimental) page cap reached: processing first {cap} of {total} textless page(s)"
    )
}

/// Emitted when Tesseract (with Vietnamese language data) is not usable.
pub(crate) fn unavailable() -> String {
    "OCR unavailable (experimental): tesseract with the 'vie' language data was not found on \
     this machine; scanned pages were not processed. Install tesseract-ocr and \
     tesseract-ocr-vie to enable it."
        .to_string()
}

/// Per-page OCR failure (timeout, non-zero exit, unreadable output).
/// Non-fatal: the remaining pages continue.
pub(crate) fn page_failed(page: u32, reason: &str) -> String {
    format!("OCR (experimental) failed on page {page}: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_sorted_and_unique() {
        assert!(OCR_LABEL_KEYS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn keys_use_ocr_camelcase_convention() {
        for key in OCR_LABEL_KEYS {
            assert!(key.starts_with("ocr."), "key {key} breaks convention");
            let body = &key["ocr.".len()..];
            assert!(
                body.chars()
                    .next()
                    .map(|c| c.is_lowercase())
                    .unwrap_or(false)
                    && !body.contains('_')
                    && !body.contains('-'),
                "key {key} is not camelCase"
            );
        }
    }

    #[test]
    fn every_builder_string_maps_to_a_listed_key() {
        let warnings: Vec<String> = vec![
            pages_via_ocr(&[1]),
            pages_via_ocr(&[3, 5, 12]),
            mean_confidence(87.25, 2),
            page_cap(50, 71),
            unavailable(),
            page_failed(4, "timeout"),
            // An arbitrary foreign warning maps to ocr.unknown, which is listed.
            "scanned pdf suspected: 3 of 3 pages have no text layer".to_string(),
        ];
        for w in warnings {
            let key = label_for_warning(&w);
            assert!(
                OCR_LABEL_KEYS.contains(&key),
                "warning {w:?} mapped to {key}, missing from OCR_LABEL_KEYS"
            );
            assert!(
                w.contains("experimental") || key == "ocr.unknown" || key == "ocr.unavailable",
                "honest-labeling contract: OCR warnings must say 'experimental' (or be the foreign/unavailable case)"
            );
        }
    }

    #[test]
    fn experimental_marker_is_present_in_ocr_provenance() {
        let w = pages_via_ocr(&[2, 9]);
        assert_eq!(w, "pages 2,9 extracted via OCR (experimental)");
        assert_eq!(label_for_warning(&w), "ocr.experimental");
    }
}
