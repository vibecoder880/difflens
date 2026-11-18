//! PDF extraction via pdfium-render with our own line/paragraph
//! reconstruction from per-character bounding boxes.
//!
//! # Reading-order reconstruction algorithm (deterministic)
//!
//! PDFium yields text as a flat per-character stream; the visual structure
//! must be rebuilt. The pipeline per page:
//!
//! 1. **Collect** every character with its tight bounding box
//!    (`PdfPageTextChar::tight_bounds`, PDF points, y-up). Characters with no
//!    usable box and control characters (including the synthetic CR/LF
//!    PDFium inserts) are dropped; whitespace characters are kept as
//!    baseline-hugging word separators only.
//! 2. **Line clustering (Y-cluster):** glyphs are sorted by descending glyph
//!    bottom (baseline proximity — vertical centres would split tiny
//!    punctuation boxes off their line) and greedily grouped: a glyph joins
//!    the current line when its baseline is within `LINE_TOLERANCE_FACTOR ×
//!    median glyph height` of the line's reference baseline. Descenders dip
//!    below the baseline by ~0.2 em, inside the tolerance.
//! 3. **X-ordering:** within a line, glyphs are sorted by left edge. A word
//!    separator is inserted when a whitespace character of this line sits
//!    between two glyphs (matched by its x-interval — its own box hugs the
//!    baseline) or when the horizontal gap exceeds `SPACE_GAP_FACTOR ×
//!    median glyph height`.
//! 4. **Paragraph merging:** consecutive lines (top → bottom) merge into one
//!    paragraph when the baseline distance is at most
//!    `PARAGRAPH_LINE_FACTOR × line height` and the left edge stays within
//!    `PARAGRAPH_INDENT_FACTOR × line height` of the previous line (a larger
//!    indent jump or a significant outdent starts a new paragraph). Lines in
//!    separate columns or rotated text are NOT handled — see the known
//!    limitation below.
//!
//! Every block records the union rectangle of its characters and its 1-based
//! page number. Pages with no text layer are counted; when more than 30% of
//! pages have no text layer a "scanned PDF" warning is emitted (feeds Phase 6
//! OCR and the UI notice).
//!
//! Known limitation (documented in the phase plan): rotated text and complex
//! multi-column layouts can produce wrong reading order; correctness is
//! gated only on axis-aligned, single-column fixtures.

use crate::error::{CoreError, CoreResult};
use crate::model::{Block, BlockKind, DocFormat, DocumentModel, Rect};

use super::ExtractedParts;
use super::normalize;
use super::pdfium_loader;
use super::read_file;

/// Vertical centre tolerance for line clustering, as a fraction of the median
/// character height.
const LINE_TOLERANCE_FACTOR: f32 = 0.5;
/// Horizontal gap (fraction of median char height) above which a word
/// separator is inserted even when PDFium emitted no whitespace character.
const SPACE_GAP_FACTOR: f32 = 0.45;
/// Vertical slack (PDF points) when deciding whether a whitespace character
/// belongs to a reconstructed line.
const SEPARATOR_Y_SLACK: f32 = 1.0;
/// Maximum baseline distance (fraction of line height) between two lines of
/// the same paragraph.
const PARAGRAPH_LINE_FACTOR: f32 = 1.6;
/// Maximum left-edge jump (fraction of line height) within one paragraph.
const PARAGRAPH_INDENT_FACTOR: f32 = 2.0;
/// Fraction of textless pages above which the document is flagged as
/// possibly scanned (Phase 2) and the OCR fallback may engage (Phase 6).
pub(crate) const SCANNED_PAGE_RATIO: f32 = 0.3;

/// Extract a PDF from disk.
pub fn extract_pdf(path: &str) -> CoreResult<DocumentModel> {
    let bytes = read_file(path)?;
    let parts = extract_pdf_bytes(path, &bytes)?;
    Ok(super::build_model(path, DocFormat::Pdf, parts))
}

/// Bind the runtime PDFium library (shared by extraction and the Phase-06
/// OCR rasterizer, which renders the same document with the same loader).
/// Returns the binding plus any loader warnings (e.g. system-library
/// fallback), which extraction must keep propagating.
pub(crate) fn bind_pdfium() -> CoreResult<(pdfium_render::prelude::Pdfium, Vec<String>)> {
    pdfium_loader::load()
        .map(|loaded| (loaded.pdfium, loaded.warnings))
        .map_err(|e| CoreError::Extraction(format!("pdfium loader: {e}")))
}

/// Open a PDF from bytes on a bound PDFium instance. The returned document
/// borrows the bytes and the binding (both must outlive it).
pub(crate) fn open_pdf<'a>(
    pdfium: &'a pdfium_render::prelude::Pdfium,
    bytes: &'a [u8],
) -> CoreResult<pdfium_render::prelude::PdfDocument<'a>> {
    pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| CoreError::Extraction(format!("failed to open pdf: {e}")))
}

/// Enumerate the pages with no usable text layer (1-based page numbers)
/// plus the total page count. Phase-06 OCR fallback uses this to decide
/// which pages to rasterize, with the same empty-text-layer criterion the
/// extractor applies when warning about scanned PDFs.
pub(crate) fn scan_textless_pages(path: &str) -> CoreResult<(Vec<u32>, u32)> {
    let bytes = read_file(path)?;
    let (pdfium, _) = bind_pdfium()?;
    let document = open_pdf(&pdfium, &bytes)?;
    let page_count = document.pages().len();
    let mut textless = Vec::new();
    for page_index in 0..page_count {
        let page = document
            .pages()
            .get(page_index)
            .map_err(|e| CoreError::Extraction(format!("page {page_index} unreadable: {e}")))?;
        let has_text = page
            .text()
            .map(|t| !collect_chars(&t).is_empty())
            .unwrap_or(false);
        if !has_text {
            textless.push(page_index as u32 + 1);
        }
    }
    Ok((textless, page_count as u32))
}

pub(crate) fn extract_pdf_bytes(_path: &str, bytes: &[u8]) -> CoreResult<ExtractedParts> {
    let (pdfium, loader_warnings) = bind_pdfium()?;
    let document = open_pdf(&pdfium, bytes)?;

    let mut warnings = loader_warnings;
    let page_count = document.pages().len();
    let mut blocks: Vec<Block> = Vec::new();
    let mut textless_pages: u32 = 0;

    for page_index in 0..page_count {
        let page = document
            .pages()
            .get(page_index)
            .map_err(|e| CoreError::Extraction(format!("page {page_index} unreadable: {e}")))?;
        let text = match page.text() {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!(
                    "page {} has no readable text layer: {e}",
                    page_index + 1
                ));
                textless_pages += 1;
                continue;
            }
        };

        let chars = collect_chars(&text);
        if chars.is_empty() {
            textless_pages += 1;
            continue;
        }

        for (line_text, rect) in page_paragraphs(&chars) {
            let text = normalize::normalize(&line_text);
            if text.is_empty() {
                continue;
            }
            blocks.push(Block {
                id: String::new(),
                kind: BlockKind::Paragraph,
                text,
                page: Some(page_index as u32 + 1),
                rect: Some(rect),
                anchor: None,
            });
        }
    }

    if page_count > 0 {
        let ratio = textless_pages as f32 / page_count as f32;
        if ratio > SCANNED_PAGE_RATIO {
            warnings.push(format!(
                "scanned pdf suspected: {textless_pages} of {page_count} pages have no text layer"
            ));
        }
    }

    Ok(super::ExtractedParts {
        blocks,
        page_count: Some(page_count as u32),
        warnings,
    })
}

/// One character with its bounding box in PDF points (y-up).
struct CharBox {
    ch: char,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

impl CharBox {
    fn centre_y(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }
    fn height(&self) -> f32 {
        self.top - self.bottom
    }
}

/// Collect all characters with usable boxes. Control characters (including
/// the synthetic CR/LF PDFium emits between lines) are dropped; whitespace
/// characters are kept — their x-intervals act as word separators (their own
/// boxes hug the baseline and are useless for Y-clustering).
fn collect_chars(text: &pdfium_render::prelude::PdfPageText<'_>) -> Vec<CharBox> {
    let mut chars = Vec::new();
    for c in text.chars().iter() {
        let Some(ch) = c.unicode_char() else { continue };
        if ch.is_control() {
            continue;
        }
        let bounds = c.tight_bounds().or_else(|_| c.loose_bounds());
        let Ok(rect) = bounds else { continue };
        let (left, right, top, bottom) = (
            rect.left().value,
            rect.right().value,
            rect.top().value,
            rect.bottom().value,
        );
        if right <= left || top <= bottom {
            continue;
        }
        chars.push(CharBox {
            ch,
            left,
            right,
            top,
            bottom,
        });
    }
    chars
}

/// A reconstructed visual line.
struct Line {
    text: String,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    height: f32,
}

/// Rebuild lines then merge them into paragraphs. Returns paragraph text
/// (lines joined with spaces) plus the union rectangle per paragraph, in
/// top-to-bottom reading order.
fn page_paragraphs(chars: &[CharBox]) -> Vec<(String, Rect)> {
    let lines = build_lines(chars);
    merge_paragraphs(lines)
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 2]
}

fn build_lines(chars: &[CharBox]) -> Vec<Line> {
    // Whitespace boxes hug the baseline (height ≈ 0) and would poison both
    // the median height and the Y-clustering, so lines are built from glyphs
    // only; whitespace characters are re-attached per line as separators.
    let glyphs: Vec<&CharBox> = chars.iter().filter(|c| !c.ch.is_whitespace()).collect();
    if glyphs.is_empty() {
        return Vec::new();
    }
    let separators: Vec<&CharBox> = chars.iter().filter(|c| c.ch.is_whitespace()).collect();

    let mut heights: Vec<f32> = glyphs
        .iter()
        .map(|c| c.height())
        .filter(|h| *h > 0.0)
        .collect();
    let median_height = median(&mut heights).max(1.0);
    let tolerance = LINE_TOLERANCE_FACTOR * median_height;

    // Cluster by baseline proximity (glyph bottom): characters on one visual
    // line share a baseline, while punctuation (., , ') has tiny glyph boxes
    // whose vertical *centre* would split it off a centre-based clustering.
    // Descenders dip below the baseline by ~0.2 em, inside the tolerance.
    // Sort by descending bottom; ties broken by left edge for determinism.
    let mut sorted: Vec<&CharBox> = glyphs;
    sorted.sort_by(|a, b| {
        b.bottom
            .total_cmp(&a.bottom)
            .then(a.left.total_cmp(&b.left))
    });

    let mut clusters: Vec<Vec<&CharBox>> = Vec::new();
    for ch in sorted {
        match clusters.last_mut() {
            Some(line) if (ch.bottom - line[0].bottom).abs() <= tolerance => {
                line.push(ch);
            }
            _ => clusters.push(vec![ch]),
        }
    }

    let mut lines = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        lines.push(build_line_text(&cluster, &separators, median_height));
    }
    lines
}

/// Sort one line's glyphs left-to-right and render them, inserting a word
/// separator where a whitespace character of this line sits between two
/// glyphs or where the inter-glyph gap alone is too wide.
fn build_line_text(cluster: &[&CharBox], separators: &[&CharBox], median_height: f32) -> Line {
    let mut glyphs: Vec<&CharBox> = cluster.to_vec();
    glyphs.sort_by(|a, b| a.left.total_cmp(&b.left));
    let gap_threshold = SPACE_GAP_FACTOR * median_height;

    let left = glyphs.iter().map(|c| c.left).fold(f32::MAX, f32::min);
    let right = glyphs.iter().map(|c| c.right).fold(f32::MIN, f32::max);
    let top = glyphs.iter().map(|c| c.top).fold(f32::MIN, f32::max);
    let bottom = glyphs.iter().map(|c| c.bottom).fold(f32::MAX, f32::min);

    // Whitespace boxes hug their line's baseline, so a separator belongs to
    // the line whose glyph bottom band it sits in — checked against `bottom`
    // (not the full extent) so spaces of the line *above* (which shares this
    // line's ascender band) cannot leak in.
    let line_seps: Vec<&CharBox> = separators
        .iter()
        .copied()
        .filter(|sep| {
            sep.centre_y() >= bottom - SEPARATOR_Y_SLACK
                && sep.centre_y() <= bottom + 0.55 * median_height
        })
        .collect();

    let mut text = String::new();
    let mut char_heights: Vec<f32> = Vec::new();

    for (i, glyph) in glyphs.iter().enumerate() {
        if i > 0 {
            let prev = glyphs[i - 1];
            let has_explicit_space = line_seps.iter().any(|sep| {
                let mid = (sep.left + sep.right) / 2.0;
                mid >= prev.right && mid <= glyph.left
            });
            let gap = glyph.left - prev.right;
            if (has_explicit_space || gap > gap_threshold) && !text.ends_with(' ') {
                text.push(' ');
            }
        }
        text.push(glyph.ch);
        char_heights.push(glyph.height());
    }

    let height = if char_heights.is_empty() {
        median_height
    } else {
        median(&mut char_heights).max(0.5)
    };
    Line {
        text,
        left,
        right,
        top,
        bottom,
        height,
    }
}

fn merge_paragraphs(lines: Vec<Line>) -> Vec<(String, Rect)> {
    let mut out: Vec<(String, Rect)> = Vec::new();
    let mut current: Option<(String, Rect, Line)> = None;

    for line in lines {
        if line.text.trim().is_empty() {
            continue;
        }
        match current.take() {
            None => current = Some((line.text.clone(), rect_of(&line), line)),
            Some((mut text, mut rect, prev)) => {
                if same_paragraph(&prev, &line) {
                    text.push(' ');
                    text.push_str(&line.text);
                    rect = union(rect, &line);
                    current = Some((text, rect, line));
                } else {
                    out.push((std::mem::take(&mut text), rect));
                    current = Some((line.text.clone(), rect_of(&line), line));
                }
            }
        }
    }
    if let Some((text, rect, _)) = current.take() {
        out.push((text, rect));
    }
    out
}

/// Paragraph-merge decision: spacing + indentation heuristics (see module
/// docs).
fn same_paragraph(prev: &Line, cur: &Line) -> bool {
    let baseline_distance = prev.top - cur.top;
    if baseline_distance <= 0.0 {
        return false;
    }
    let height = prev.height.max(cur.height);
    let spacing_ok = baseline_distance <= PARAGRAPH_LINE_FACTOR * height;
    let indent_ok = cur.left >= prev.left - 0.5 * height
        && cur.left <= prev.left + PARAGRAPH_INDENT_FACTOR * height;
    spacing_ok && indent_ok
}

fn rect_of(line: &Line) -> Rect {
    Rect {
        x: line.left,
        y: line.bottom,
        width: line.right - line.left,
        height: line.top - line.bottom,
    }
}

fn union(rect: Rect, line: &Line) -> Rect {
    let l = rect.x.min(line.left);
    let r = (rect.x + rect.width).max(line.right);
    let b = rect.y.min(line.bottom);
    let t = (rect.y + rect.height).max(line.top);
    Rect {
        x: l,
        y: b,
        width: r - l,
        height: t - b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic glyph: bottom = baseline, height ≈ 7 for x-height glyphs.
    fn glyph_width(ch: char) -> f32 {
        match ch {
            'i' | 'l' | '.' => 1.0,
            _ => 5.0,
        }
    }

    fn glyph(ch: char, left: f32, baseline: f32) -> CharBox {
        let height = if ch == '.' { 1.5 } else { 7.0 };
        CharBox {
            ch,
            left,
            right: left + glyph_width(ch),
            top: baseline + height,
            bottom: baseline,
        }
    }

    /// Place a word left-to-right starting at `x`; returns the next x.
    fn place_word(chars: &mut Vec<CharBox>, word: &str, x0: f32, baseline: f32) -> f32 {
        let mut x = x0;
        for ch in word.chars() {
            chars.push(glyph(ch, x, baseline));
            x += glyph_width(ch) + 0.5;
        }
        x
    }

    /// Synthetic space: baseline-hugging box, like PDFium emits.
    fn space(left: f32, baseline: f32) -> CharBox {
        CharBox {
            ch: ' ',
            left,
            right: left + 3.3,
            top: baseline + 0.01,
            bottom: baseline,
        }
    }

    #[test]
    fn punctuation_stays_on_its_line() {
        // "cat." on one baseline: the period's tiny box must not split off.
        let mut chars = Vec::new();
        let x = place_word(&mut chars, "cat", 10.0, 100.0);
        chars.push(glyph('.', x, 100.0));
        let lines = build_lines(&chars);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "cat.");
    }

    #[test]
    fn two_lines_cluster_separately_and_words_get_spaces() {
        let mut chars = Vec::new();
        let x = place_word(&mut chars, "two", 10.0, 100.0);
        chars.push(space(x, 100.0));
        let _ = place_word(&mut chars, "lines", x + 3.8, 100.0);
        // Second baseline 12 points below the first (well past tolerance).
        place_word(&mut chars, "next", 10.0, 88.0);
        let lines = build_lines(&chars);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "two lines");
        assert_eq!(lines[1].text, "next");
    }

    #[test]
    fn close_baselines_merge_into_one_paragraph() {
        let mut chars = Vec::new();
        place_word(&mut chars, "one", 10.0, 100.0);
        place_word(&mut chars, "two", 10.0, 90.0);
        let paras = page_paragraphs(&chars);
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].0, "one two");
        assert!(paras[0].1.height > 14.0, "paragraph rect spans both lines");
    }

    #[test]
    fn wide_baseline_gap_starts_new_paragraph() {
        let mut chars = Vec::new();
        place_word(&mut chars, "one", 10.0, 100.0);
        place_word(&mut chars, "two", 10.0, 70.0);
        let paras = page_paragraphs(&chars);
        assert_eq!(paras.len(), 2);
    }
}
