//! DOCX extraction: raw `zip` + `roxmltree` parse of `word/document.xml`.
//!
//! A DOCX is a zip container; we deliberately do not use a docx crate
//! (research §2.3: diff-grade fidelity — exact run ordering, `w:ins`/`w:del`
//! handling, tables — is not safely reachable through writer-oriented
//! models). Mapped markup:
//!
//! - `w:body > w:p` → paragraph block; runs `w:r > w:t` in document order,
//!   `w:tab` → tab, `w:br`/`w:cr` → newline, `w:hyperlink` inner runs
//!   included.
//! - `w:pPr > w:pStyle` with `Heading1`–`Heading6` → heading block.
//! - Tracked changes (accepted view, see [`TrackedChangesMode`]): text in
//!   `w:ins` counts as content; text in `w:del > w:delText` is excluded from
//!   the block text but preserved in the block's `anchor` field so a later
//!   phase can surface it.
//! - `w:body > w:tbl` → table block (`rows` × `cols`, cell text joined
//!   row-major with tab/newline separators).
//! - Headers, footers, footnotes, endnotes are skipped with a warning.
//!
//! Resource limits (entry count, decompressed size) are enforced before
//! parsing via [`super::ZipBudget`].

use crate::error::{CoreError, CoreResult};
use crate::model::{Block, BlockKind, DocFormat, DocumentModel};

use super::normalize;
use super::{ExtractedParts, ZipBudget, read_file};
use std::io::{Cursor, Read, Seek};
use zip::ZipArchive;

/// WordprocessingML namespace.
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// Ceiling on `word/document.xml` itself — anything larger is treated as a
/// zip bomb even though the whole-archive budget is higher.
const MAX_DOCUMENT_XML_BYTES: u64 = 64 * 1024 * 1024;

/// How tracked changes (revisions) are rendered.
///
/// Phase 2 only wires [`TrackedChangesMode::Accepted`]; the toggle is
/// exposed to the UI in a later phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackedChangesMode {
    /// `w:ins` text kept, `w:del` text excluded (document "as accepted").
    #[default]
    Accepted,
    /// Deleted text included (document "as marked up"); not wired yet.
    Original,
}

/// Extract a DOCX from disk.
pub fn extract_docx(path: &str) -> CoreResult<DocumentModel> {
    let bytes = read_file(path)?;
    let parts = extract_docx_bytes(path, &bytes)?;
    Ok(super::build_model(path, DocFormat::Docx, parts))
}

pub(crate) fn extract_docx_bytes(_path: &str, bytes: &[u8]) -> CoreResult<ExtractedParts> {
    extract_container(bytes, TrackedChangesMode::Accepted)
}

fn extract_container(bytes: &[u8], mode: TrackedChangesMode) -> CoreResult<ExtractedParts> {
    let mut zip = open_archive(bytes)?;
    let mut budget = ZipBudget::new(super::MAX_ZIP_UNCOMPRESSED_BYTES);

    let document_xml = super::read_zip_entry_limited(
        &mut zip,
        "word/document.xml",
        &mut budget,
        MAX_DOCUMENT_XML_BYTES,
    )?
    .ok_or_else(|| CoreError::Extraction("docx is missing word/document.xml".to_string()))?;
    let xml = std::str::from_utf8(&document_xml)
        .map_err(|_| CoreError::Extraction("word/document.xml is not UTF-8".to_string()))?;
    let tree = roxmltree::Document::parse(xml)
        .map_err(|e| CoreError::Extraction(format!("malformed word/document.xml: {e}")))?;

    let body = tree
        .root_element()
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "body")
        .ok_or_else(|| CoreError::Extraction("word/document.xml has no w:body".to_string()))?;

    let mut blocks: Vec<Block> = Vec::new();
    for node in body.children().filter(|n| n.is_element()) {
        let tag = node.tag_name().name();
        match tag {
            "p" => {
                let info = paragraph_info(&node, mode);
                push_paragraph(&mut blocks, info, None);
            }
            "tbl" => {
                if let Some(block) = table_block(&node) {
                    blocks.push(block);
                }
            }
            _ => {}
        }
    }

    let warnings = container_warnings(&mut zip);

    Ok(ExtractedParts {
        blocks,
        page_count: None,
        warnings,
    })
}

fn open_archive(bytes: &[u8]) -> CoreResult<ZipArchive<Cursor<&[u8]>>> {
    let zip = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| CoreError::Extraction(format!("unreadable docx container: {e}")))?;
    if zip.len() > super::MAX_ZIP_ENTRIES {
        return Err(CoreError::Extraction(format!(
            "docx exceeds the {} entry limit",
            super::MAX_ZIP_ENTRIES
        )));
    }
    Ok(zip)
}

/// Count ancillary parts we intentionally skip so the UI can inform the user.
fn container_warnings<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Vec<String> {
    let mut warnings = Vec::new();
    let skipped: usize = zip
        .file_names()
        .filter_map(|n| n.ok())
        .filter(|n| {
            n.starts_with("word/header")
                || n.starts_with("word/footer")
                || n == "word/footnotes.xml"
                || n == "word/endnotes.xml"
        })
        .count();
    if skipped > 0 {
        warnings.push(format!("skipped {skipped} header/footer/footnote part(s)"));
    }
    warnings
}

/// Text of one paragraph (or one table cell): runs in document order, with
/// tracked-changes handling and tab/break mapping.
struct ParaText {
    text: String,
    deleted: String,
    heading_level: Option<u8>,
}

fn paragraph_info(p: &roxmltree::Node, mode: TrackedChangesMode) -> ParaText {
    let mut out = ParaText {
        text: String::new(),
        deleted: String::new(),
        heading_level: None,
    };

    if let Some(style) = p
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "pPr")
        .and_then(|ppr| {
            ppr.children()
                .find(|n| n.is_element() && n.tag_name().name() == "pStyle")
        })
        .and_then(|ps| ps.attribute((W, "val")))
    {
        out.heading_level = heading_level_from_style(style);
    }

    for node in p.descendants().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "t" => {
                if let Some(t) = node.text() {
                    out.text.push_str(t);
                }
            }
            "delText" => {
                if let Some(t) = node.text() {
                    out.deleted.push_str(t);
                    if mode == TrackedChangesMode::Original {
                        out.text.push_str(t);
                    }
                }
            }
            "tab" => out.text.push('\t'),
            "br" | "cr" => out.text.push('\n'),
            _ => {}
        }
    }
    out
}

/// `Heading1` … `Heading6` style ids (case-insensitive, whitespace-tolerant).
/// Localized style names ("Titre 1") are not recognized — a known
/// degradation, surfaced as a plain paragraph rather than a failure.
fn heading_level_from_style(style: &str) -> Option<u8> {
    let compact: String = style.chars().filter(|c| !c.is_whitespace()).collect();
    let lower = compact.to_ascii_lowercase();
    let digits = lower.strip_prefix("heading")?;
    if digits.len() != 1 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let level = digits.parse::<u8>().ok()?;
    (1..=6).contains(&level).then_some(level)
}

fn push_paragraph(blocks: &mut Vec<Block>, info: ParaText, anchor_override: Option<String>) {
    let text = normalize::normalize(&info.text);
    if text.is_empty() {
        return;
    }
    let deleted = normalize::normalize(&info.deleted);
    let anchor = anchor_override.or(if deleted.is_empty() {
        None
    } else {
        Some(deleted)
    });
    let kind = match info.heading_level {
        Some(level) => BlockKind::Heading { level },
        None => BlockKind::Paragraph,
    };
    blocks.push(Block {
        id: String::new(),
        kind,
        text,
        page: None,
        rect: None,
        anchor,
    });
}

fn table_block(tbl: &roxmltree::Node) -> Option<Block> {
    let rows: Vec<roxmltree::Node> = tbl
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "tr")
        .collect();
    if rows.is_empty() {
        return None;
    }

    let mut row_texts: Vec<String> = Vec::with_capacity(rows.len());
    let mut cols = 0usize;
    for row in &rows {
        let cells: Vec<roxmltree::Node> = row
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "tc")
            .collect();
        cols = cols.max(cells.len());
        let cell_texts: Vec<String> = cells
            .iter()
            .map(|cell| {
                normalize::normalize(&paragraph_info(cell, TrackedChangesMode::Accepted).text)
            })
            .filter(|t| !t.is_empty())
            .collect();
        row_texts.push(cell_texts.join("\t"));
    }

    let text = normalize::normalize(&row_texts.join("\n"));
    if text.is_empty() {
        return None;
    }
    Some(Block {
        id: String::new(),
        kind: BlockKind::Table {
            rows: rows.len(),
            cols,
        },
        text,
        page: None,
        rect: None,
        anchor: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_style_parsing() {
        assert_eq!(heading_level_from_style("Heading1"), Some(1));
        assert_eq!(heading_level_from_style(" heading 3 "), Some(3));
        assert_eq!(heading_level_from_style("HEADING6"), Some(6));
        assert_eq!(heading_level_from_style("Heading7"), None);
        assert_eq!(heading_level_from_style("Heading"), None);
        assert_eq!(heading_level_from_style("Title"), None);
        assert_eq!(heading_level_from_style("ListParagraph"), None);
    }

    #[test]
    fn tracked_changes_mode_default_is_accepted() {
        assert_eq!(TrackedChangesMode::default(), TrackedChangesMode::Accepted);
    }
}
