//! PPTX extraction: raw `zip` + `roxmltree` over `ppt/slides/slideN.xml`
//! (ordered numerically — the research §2.4 pattern; no maintained pptx
//! crate exists).
//!
//! Each slide becomes a single [`BlockKind::Slide`] block whose text is the
//! concatenation of all `<a:t>` runs in document order (paragraphs joined
//! with newlines). When the slide's title placeholder (`p:ph` type `title` /
//! `ctrTitle`) is detectable its text is stored in the block's `anchor`.
//! Speaker notes (`ppt/notesSlides/`) are skipped with a warning.

use crate::error::{CoreError, CoreResult};
use crate::model::{Block, BlockKind, DocFormat, DocumentModel};

use super::normalize;
use super::{ExtractedParts, ZipBudget, read_file};
use std::io::Cursor;
use zip::ZipArchive;

/// DrawingML namespace (`a:t` runs, `a:p` paragraphs).
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// PresentationML namespace (`p:sp` shapes, `p:ph` placeholders).
const P_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

/// Extract a PPTX from disk.
pub fn extract_pptx(path: &str) -> CoreResult<DocumentModel> {
    let bytes = read_file(path)?;
    let parts = extract_pptx_bytes(path, &bytes)?;
    Ok(super::build_model(path, DocFormat::Pptx, parts))
}

pub(crate) fn extract_pptx_bytes(_path: &str, bytes: &[u8]) -> CoreResult<ExtractedParts> {
    let mut zip = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| CoreError::Extraction(format!("unreadable pptx container: {e}")))?;
    if zip.len() > super::MAX_ZIP_ENTRIES {
        return Err(CoreError::Extraction(format!(
            "pptx exceeds the {} entry limit",
            super::MAX_ZIP_ENTRIES
        )));
    }
    let mut budget = ZipBudget::new(super::MAX_ZIP_UNCOMPRESSED_BYTES);

    let mut slides: Vec<(u32, String)> = Vec::new();
    for name in zip.file_names().filter_map(|n| n.ok()) {
        if let Some(n) = slide_number(&name) {
            slides.push((n, name.to_string()));
        }
    }
    slides.sort_by_key(|(n, _)| *n);

    let notes = zip
        .file_names()
        .filter_map(|n| n.ok())
        .filter(|n| n.starts_with("ppt/notesSlides/") && n.ends_with(".xml"))
        .count();
    let mut warnings = Vec::new();
    if notes > 0 {
        warnings.push(format!("skipped {notes} speaker-notes slide(s)"));
    }

    if slides.is_empty() {
        return Err(CoreError::Extraction("pptx contains no slides".to_string()));
    }

    let mut blocks: Vec<Block> = Vec::new();
    for (position, (_, name)) in slides.iter().enumerate() {
        let xml_bytes = super::read_zip_entry_limited(
            &mut zip,
            name,
            &mut budget,
            super::MAX_SINGLE_PART_BYTES,
        )?
        .ok_or_else(|| CoreError::Extraction(format!("missing slide part {name}")))?;
        let xml = match std::str::from_utf8(&xml_bytes) {
            Ok(s) => s,
            Err(_) => {
                warnings.push(format!("slide part {name} is not UTF-8 and was skipped"));
                continue;
            }
        };
        let tree = match roxmltree::Document::parse(xml) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!(
                    "slide part {name} is malformed and was skipped: {e}"
                ));
                continue;
            }
        };

        let root = tree.root_element();
        let title = slide_title(&root);
        let text = normalize::normalize(&slide_text(&root));
        if text.is_empty() {
            continue;
        }
        blocks.push(Block {
            id: String::new(),
            // `index` is 0-based within the deck; `page` is the 1-based slide
            // number (they agree because slides are ordered numerically).
            kind: BlockKind::Slide {
                index: position as u32,
            },
            text,
            page: Some(position as u32 + 1),
            rect: None,
            anchor: title,
        });
    }

    Ok(ExtractedParts {
        blocks,
        page_count: Some(slides.len() as u32),
        warnings,
    })
}

/// `ppt/slides/slideN.xml` → N (1-based), any other name → None.
fn slide_number(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("ppt/slides/slide")?;
    let digits = rest.strip_suffix(".xml")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok()
}

/// Text of one slide: all `a:t` runs in document order, paragraphs (`a:p`)
/// joined with newlines.
fn slide_text(root: &roxmltree::Node) -> String {
    let mut out = String::new();
    for paragraph in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name((A_NS, "p")))
    {
        for run in paragraph
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name((A_NS, "t")))
        {
            if let Some(t) = run.text() {
                out.push_str(t);
            }
        }
        out.push('\n');
    }
    out
}

/// Title text when the slide has a `title` / `ctrTitle` placeholder.
fn slide_title(root: &roxmltree::Node) -> Option<String> {
    for shape in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name((P_NS, "sp")))
    {
        let is_title = shape.descendants().any(|n| {
            n.is_element()
                && n.has_tag_name((P_NS, "ph"))
                && n.attributes()
                    .find(|a| a.name() == "type")
                    .map(|a| a.value())
                    .map(|t| t == "title" || t == "ctrTitle")
                    .unwrap_or(false)
        });
        if !is_title {
            continue;
        }
        let mut text = String::new();
        for run in shape
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name((A_NS, "t")))
        {
            if let Some(t) = run.text() {
                text.push_str(t);
            }
        }
        let normalized = normalize::normalize(&text);
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slide_numbers_parse() {
        assert_eq!(slide_number("ppt/slides/slide1.xml"), Some(1));
        assert_eq!(slide_number("ppt/slides/slide12.xml"), Some(12));
        assert_eq!(slide_number("ppt/slides/slideN.xml"), None);
        assert_eq!(slide_number("ppt/notesSlides/notesSlide1.xml"), None);
        assert_eq!(slide_number("ppt/slides/slide.xml"), None);
        assert_eq!(slide_number("ppt/slides/slide-1.xml"), None);
    }
}
