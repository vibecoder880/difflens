//! Extraction layer: turn a file into an ordered [`DocumentModel`] of blocks.
//!
//! Entry point: [`extract`]. Dispatching is on content sniffing (magic
//! bytes + OOXML part presence), not on file extensions; extension
//! mismatches produce a warning instead of an error. Every extractor degrades
//! gracefully: exotic/unsupported parts produce warnings, never panics.
//!
//! Resource limits (zip bombs, oversized parts) are enforced before parsing.
//! 100% local: no network access anywhere in this module.

mod docx;
pub mod normalize;
// `pub(crate)`: the Phase-06 OCR fallback shares this module's pdfium
// binding and textless-page scan.
pub(crate) mod pdf;
// `pub`: the Tauri shell reads the bundled-resource env var names
// (DIFFLENS_RESOURCE_DIR) to seed the packaged-app loader; the loader
// functions themselves stay `pub(crate)`.
pub mod pdfium_loader;
mod pptx;
mod txt;
mod xlsx;

pub use docx::{TrackedChangesMode, extract_docx};
pub use pdf::extract_pdf;
pub use pptx::extract_pptx;
pub use txt::extract_txt;
pub use xlsx::extract_xlsx;

use crate::error::{CoreError, CoreResult};
use crate::model::{Block, DocFormat, DocumentModel};
use std::io::{Cursor, Read};
use zip::ZipArchive;

/// Maximum number of zip entries in a DOCX/PPTX container.
pub(crate) const MAX_ZIP_ENTRIES: usize = 10_000;
/// Maximum total decompressed bytes read out of one container.
pub(crate) const MAX_ZIP_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum size for a single container part we read.
pub(crate) const MAX_SINGLE_PART_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum rows per spreadsheet sheet (plan cap; truncation warns).
pub(crate) const MAX_XLSX_ROWS: usize = 100_000;
/// Maximum size of a plain-text document.
pub(crate) const MAX_TXT_BYTES: usize = 64 * 1024 * 1024;

/// Blocks, page count and warnings produced by a format extractor before
/// ids are assigned and the model is assembled.
pub(crate) struct ExtractedParts {
    pub blocks: Vec<Block>,
    pub page_count: Option<u32>,
    pub warnings: Vec<String>,
}

/// Extract any supported document (PDF, DOCX, TXT/MD, XLSX, PPTX).
pub fn extract(path: &str) -> CoreResult<DocumentModel> {
    let bytes = read_file(path)?;
    let format = sniff_format(&bytes)?;

    let mut parts = match format {
        DocFormat::Pdf => pdf::extract_pdf_bytes(path, &bytes)?,
        DocFormat::Docx => docx::extract_docx_bytes(path, &bytes)?,
        DocFormat::Xlsx => xlsx::extract_xlsx_bytes(path, &bytes)?,
        DocFormat::Pptx => pptx::extract_pptx_bytes(path, &bytes)?,
        DocFormat::Txt => txt::extract_txt_bytes(path, &bytes)?,
    };

    let ext_format = DocFormat::from_extension(
        std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default(),
    );
    if let Some(ext_format) = ext_format
        && ext_format != format
    {
        parts.warnings.push(format!(
            "extension suggests {ext_format:?} but content sniffed as {format:?}"
        ));
    }

    Ok(build_model(path, format, parts))
}

fn read_file(path: &str) -> CoreResult<Vec<u8>> {
    std::fs::read(path).map_err(|e| CoreError::Extraction(format!("cannot read {path}: {e}")))
}

/// Content sniffing:
/// - `PK\x03\x04` → OOXML container, resolved by part presence
///   (`word/document.xml`, `xl/workbook.xml`, `ppt/presentation.xml`).
/// - `%PDF-` → PDF.
/// - everything else → text (empty files are valid empty text documents).
fn sniff_format(bytes: &[u8]) -> CoreResult<DocFormat> {
    if bytes.starts_with(b"PK\x03\x04") {
        return sniff_container(bytes);
    }
    if bytes.starts_with(b"%PDF-") {
        return Ok(DocFormat::Pdf);
    }
    Ok(DocFormat::Txt)
}

fn sniff_container(bytes: &[u8]) -> CoreResult<DocFormat> {
    let archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| CoreError::Extraction(format!("unreadable zip container: {e}")))?;
    let has = |name: &str| archive.index_for_name(name).is_some();
    if has("word/document.xml") {
        return Ok(DocFormat::Docx);
    }
    if has("xl/workbook.xml") {
        return Ok(DocFormat::Xlsx);
    }
    if has("ppt/presentation.xml")
        || archive
            .file_names()
            .filter_map(|n| n.ok())
            .any(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
    {
        return Ok(DocFormat::Pptx);
    }
    Err(CoreError::Extraction(
        "zip container is not a supported OOXML document".to_string(),
    ))
}

/// Decompressed-bytes budget shared across all parts read from one container.
pub(crate) struct ZipBudget {
    remaining: u64,
}

impl ZipBudget {
    pub(crate) fn new(max: u64) -> Self {
        Self { remaining: max }
    }

    fn reserve(&mut self, declared: u64) -> CoreResult<()> {
        if declared > self.remaining {
            return Err(CoreError::Extraction(format!(
                "zip bomb suspected: declared decompressed size {declared} bytes exceeds the \
                 remaining budget of {} bytes",
                self.remaining
            )));
        }
        self.remaining -= declared;
        Ok(())
    }
}

/// Read one entry, enforcing the declared and actual decompressed size
/// against `budget` and a per-part ceiling. Returns `Ok(None)` when the part
/// is absent.
pub(crate) fn read_zip_entry_limited<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    budget: &mut ZipBudget,
    part_ceiling: u64,
) -> CoreResult<Option<Vec<u8>>> {
    let index = match archive.index_for_name(name) {
        Some(i) => i,
        None => return Ok(None),
    };
    let mut file = archive
        .by_index(index)
        .map_err(|e| CoreError::Extraction(format!("zip part {name} unreadable: {e}")))?;
    let declared = file.size();
    if declared > part_ceiling {
        return Err(CoreError::Extraction(format!(
            "zip part {name} declares {declared} bytes, above the {part_ceiling}-byte part ceiling"
        )));
    }
    budget.reserve(declared)?;

    // Read with a guard one byte past the declared size so a lying header
    // cannot inflate memory usage.
    let mut guarded = (&mut file).take(declared + 1);
    let mut out = Vec::with_capacity(declared.min(8 * 1024 * 1024) as usize);
    guarded
        .read_to_end(&mut out)
        .map_err(|e| CoreError::Extraction(format!("zip part {name} decompression failed: {e}")))?;
    if out.len() as u64 > declared {
        return Err(CoreError::Extraction(format!(
            "zip bomb suspected: zip part {name} decompressed to more than its declared size"
        )));
    }
    Ok(Some(out))
}

/// Assign stable per-document ids (`b0`, `b1`, …) in block order and assemble
/// the final model.
pub(crate) fn build_model(path: &str, format: DocFormat, parts: ExtractedParts) -> DocumentModel {
    let ExtractedParts {
        mut blocks,
        page_count,
        warnings,
    } = parts;
    for (i, block) in blocks.iter_mut().enumerate() {
        block.id = format!("b{i}");
    }
    DocumentModel {
        path: path.to_string(),
        format,
        blocks,
        page_count,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_detects_formats() {
        assert_eq!(sniff_format(b"%PDF-1.4 rest").unwrap(), DocFormat::Pdf);
        assert_eq!(sniff_format(b"plain text").unwrap(), DocFormat::Txt);
        assert_eq!(sniff_format(b"").unwrap(), DocFormat::Txt);
        assert!(sniff_format(b"PK\x03\x04garbage").is_err());
    }

    #[test]
    fn budget_rejects_overdeclared_entries() {
        let mut budget = ZipBudget::new(1024);
        assert!(budget.reserve(100).is_ok());
        assert!(budget.reserve(1000).is_err());
    }

    #[test]
    fn ids_are_stable_and_ordered() {
        let parts = ExtractedParts {
            blocks: vec![
                Block {
                    id: String::new(),
                    kind: crate::model::BlockKind::Paragraph,
                    text: "a".into(),
                    page: None,
                    rect: None,
                    anchor: None,
                },
                Block {
                    id: String::new(),
                    kind: crate::model::BlockKind::Paragraph,
                    text: "b".into(),
                    page: None,
                    rect: None,
                    anchor: None,
                },
            ],
            page_count: None,
            warnings: Vec::new(),
        };
        let model = build_model("x.txt", DocFormat::Txt, parts);
        assert_eq!(model.blocks[0].id, "b0");
        assert_eq!(model.blocks[1].id, "b1");
    }
}
