//! OCR engine: run the detected Tesseract binary per rasterized page,
//! parse its word-level TSV output and rebuild reading-order blocks.
//!
//! # TSV parsing
//!
//! `tesseract <image> stdout -l <langs> --psm 3 tsv` emits tab-separated
//! rows with 12 columns (a header line included):
//! `level page block par line word left top width height conf text`.
//! Only level-5 (word) rows carry usable boxes; malformed rows (short,
//! non-numeric, negative confidence, empty text) are skipped — a garbled
//! row must never fail the page.
//!
//! # Line/paragraph reconstruction (documented port of `extract/pdf.rs`)
//!
//! The Phase-02 extractor Y-clusters *per-character* boxes in PDF points
//! (y-up). Tesseract gives *whole-word* boxes in raster pixels (y-down),
//! which changes two things: words already carry their spaces (no gap
//! synthesis needed), and the vertical axis is flipped. The clustering
//! idea is identical — sort by baseline proximity (box bottom), greedily
//! group within `0.5 × median box height`, order each line by left edge,
//! then merge consecutive lines into paragraphs with the same
//! spacing/indent factors — but the axis and token granularity differ, so
//! this is a documented port rather than a shared generic helper: an
//! abstraction parametrized over axis direction and token kind would be
//! harder to read than the two focused implementations. If both drift
//! apart in a future phase, revisit.
//!
//! Rects are converted back to PDF points with a y-up origin (matching
//! `extract/pdf.rs`): `rect.y` is the distance from the page bottom to the
//! box bottom, so downstream move-delta math shares one convention.
//! Every OCR block carries `anchor: Some("ocr")` as honest provenance.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{CoreError, CoreResult};
use crate::model::{Block, BlockKind, Rect};

use super::detect::OcrAvailability;
use super::proc::run_with_timeout;
use super::rasterize::{RasterizedPage, render_page};

/// Column count of a Tesseract TSV row.
const TSV_COLUMNS: usize = 12;
/// TSV level for word rows.
const TSV_LEVEL_WORD: i64 = 5;
/// Vertical tolerance for line clustering, as a fraction of the median word
/// box height (mirrors `extract/pdf.rs::LINE_TOLERANCE_FACTOR`).
const LINE_TOLERANCE_FACTOR: f32 = 0.5;
/// Maximum baseline distance (fraction of line height) between two lines of
/// the same paragraph (mirrors `extract/pdf.rs::PARAGRAPH_LINE_FACTOR`).
const PARAGRAPH_LINE_FACTOR: f32 = 1.6;
/// Maximum left-edge jump (fraction of line height) within one paragraph
/// (mirrors `extract/pdf.rs::PARAGRAPH_INDENT_FACTOR`).
const PARAGRAPH_INDENT_FACTOR: f32 = 2.0;

/// One recognized word with its raster-pixel box (top-down origin).
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub left: i64,
    pub top: i64,
    pub width: i64,
    pub height: i64,
    /// Tesseract confidence, 0..=100.
    pub conf: f32,
    pub text: String,
}

/// Parse Tesseract TSV output into recognized words. Malformed rows are
/// skipped (robustness contract): wrong column count, non-numeric fields,
/// negative confidence (tesseract emits -1 on non-word rows) and empty
/// text.
pub fn parse_tsv(tsv: &str) -> Vec<OcrWord> {
    let mut words = Vec::new();
    for line in tsv.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < TSV_COLUMNS {
            continue;
        }
        // level page block par line word left top width height conf text
        let level: i64 = match cols[0].trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if level != TSV_LEVEL_WORD {
            continue;
        }
        let left = match cols[6].trim().parse::<i64>() {
            Ok(v) if v >= 0 => v,
            _ => continue,
        };
        let top = match cols[7].trim().parse::<i64>() {
            Ok(v) if v >= 0 => v,
            _ => continue,
        };
        let width = match cols[8].trim().parse::<i64>() {
            Ok(v) if v > 0 => v,
            _ => continue,
        };
        let height = match cols[9].trim().parse::<i64>() {
            Ok(v) if v > 0 => v,
            _ => continue,
        };
        let conf = match cols[10].trim().parse::<f32>() {
            Ok(v) if v >= 0.0 && v.is_finite() => v,
            _ => continue,
        };
        let text = cols[11].trim();
        if text.is_empty() {
            continue;
        }
        words.push(OcrWord {
            left,
            top,
            width,
            height,
            conf,
            text: text.to_string(),
        });
    }
    words
}

/// A reconstructed line of words (raster pixels, y-down).
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
    pub height: f32,
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 2]
}

/// Y-cluster words into lines (top-to-bottom), then order each line by its
/// left edge and join with single spaces (Tesseract words are already
/// whitespace-tokenized). Documented y-down port of
/// `extract/pdf.rs::build_lines`.
pub fn words_to_lines(words: &[OcrWord]) -> Vec<OcrLine> {
    if words.is_empty() {
        return Vec::new();
    }
    let median_height =
        median(&mut words.iter().map(|w| w.height as f32).collect::<Vec<_>>()).max(1.0);
    let tolerance = LINE_TOLERANCE_FACTOR * median_height;

    // Baseline proximity: in y-down pixels the box bottom grows downward, so
    // ascending bottom = topmost line first. Ties by left edge (same
    // deterministic tie-break as the y-up extractor).
    let mut sorted: Vec<&OcrWord> = words.iter().collect();
    sorted.sort_by(|a, b| {
        (a.top + a.height)
            .cmp(&(b.top + b.height))
            .then(a.left.cmp(&b.left))
    });

    let mut clusters: Vec<Vec<&OcrWord>> = Vec::new();
    for word in sorted {
        match clusters.last_mut() {
            Some(line) => {
                let reference_bottom = line[0].top + line[0].height;
                let bottom = word.top + word.height;
                if (bottom - reference_bottom).abs() as f32 <= tolerance {
                    line.push(word);
                } else {
                    clusters.push(vec![word]);
                }
            }
            None => clusters.push(vec![word]),
        }
    }

    clusters
        .into_iter()
        .map(|mut cluster| {
            cluster.sort_by_key(|w| w.left);
            let text = cluster
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let left = cluster.iter().map(|w| w.left).min().unwrap_or(0);
            let top = cluster.iter().map(|w| w.top).min().unwrap_or(0);
            let right = cluster.iter().map(|w| w.left + w.width).max().unwrap_or(0);
            let bottom = cluster.iter().map(|w| w.top + w.height).max().unwrap_or(0);
            let height =
                median(&mut cluster.iter().map(|w| w.height as f32).collect::<Vec<_>>()).max(1.0);
            OcrLine {
                text,
                left,
                top,
                right,
                bottom,
                height,
            }
        })
        .collect()
}

/// Rect in raster pixels (y-down), the natural OCR coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectPx {
    pub left: i64,
    pub top: i64,
    pub width: i64,
    pub height: i64,
}

fn rect_of(line: &OcrLine) -> RectPx {
    RectPx {
        left: line.left,
        top: line.top,
        width: line.right - line.left,
        height: line.bottom - line.top,
    }
}

fn union(rect: RectPx, line: &OcrLine) -> RectPx {
    let l = rect.left.min(line.left);
    let t = rect.top.min(line.top);
    let r = (rect.left + rect.width).max(line.right);
    let b = (rect.top + rect.height).max(line.bottom);
    RectPx {
        left: l,
        top: t,
        width: r - l,
        height: b - t,
    }
}

/// Paragraph-merge decision in y-down pixels (mirrors
/// `extract/pdf.rs::same_paragraph` with the axis flipped: "below" means a
/// larger top).
fn same_paragraph(prev_top: i64, prev_left: i64, prev_height: f32, cur: &OcrLine) -> bool {
    let baseline_distance = (cur.top - prev_top) as f32;
    if baseline_distance <= 0.0 {
        return false;
    }
    let height = prev_height.max(cur.height);
    let spacing_ok = baseline_distance <= PARAGRAPH_LINE_FACTOR * height;
    let indent_ok = (cur.left as f32) >= prev_left as f32 - 0.5 * height
        && (cur.left as f32) <= prev_left as f32 + PARAGRAPH_INDENT_FACTOR * height;
    spacing_ok && indent_ok
}

/// Reconstruct full blocks from words: Y-cluster lines, merge them into
/// paragraphs, convert pixel rects back to y-up PDF-point rects and tag
/// every block with `anchor: Some("ocr")`.
pub fn words_to_blocks(
    words: &[OcrWord],
    page_no: u32,
    page_height_px: f32,
    dpi: f32,
) -> Vec<Block> {
    let lines = words_to_lines(words);
    let px_to_pt = 72.0 / dpi.max(1.0);
    let mut blocks = Vec::new();
    let mut current: Option<(String, RectPx)> = None;

    for line in lines {
        if line.text.trim().is_empty() {
            continue;
        }
        match current.take() {
            None => current = Some((line.text.clone(), rect_of(&line))),
            Some((mut text, mut rect)) => {
                let prev_height = (rect.height.max(1)) as f32;
                if same_paragraph(rect.top, rect.left, prev_height, &line) {
                    text.push(' ');
                    text.push_str(&line.text);
                    rect = union(rect, &line);
                    current = Some((text, rect));
                } else {
                    blocks.push(make_block(&text, &rect, page_no, page_height_px, px_to_pt));
                    current = Some((line.text.clone(), rect_of(&line)));
                }
            }
        }
    }
    if let Some((text, rect)) = current.take() {
        blocks.push(make_block(&text, &rect, page_no, page_height_px, px_to_pt));
    }
    blocks
}

fn make_block(
    text: &str,
    rect: &RectPx,
    page_no: u32,
    page_height_px: f32,
    px_to_pt: f32,
) -> Block {
    // y-up origin (distance from page bottom to box bottom), matching
    // extract/pdf.rs so downstream geometry consumers share one convention.
    let bottom_y_up = page_height_px - (rect.top + rect.height).max(0) as f32;
    Block {
        id: String::new(),
        kind: BlockKind::Paragraph,
        text: crate::extract::normalize::normalize(text),
        page: Some(page_no),
        rect: Some(Rect {
            x: rect.left as f32 * px_to_pt,
            y: bottom_y_up * px_to_pt,
            width: rect.width.max(0) as f32 * px_to_pt,
            height: rect.height.max(0) as f32 * px_to_pt,
        }),
        anchor: Some("ocr".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Page-level orchestration
// ---------------------------------------------------------------------------

/// Result of OCR over a set of pages.
pub struct OcrOutcome {
    pub blocks: Vec<Block>,
    /// 1-based pages that produced at least one block.
    pub pages_done: Vec<u32>,
    /// Per-page failure warnings (built via the labels module). Document
    /// extraction never fails because of OCR.
    pub warnings: Vec<String>,
    /// Mean word confidence across all recognized words (0 when none).
    pub mean_conf: f32,
}

/// OCR the given 1-based `pages` of a PDF: rasterize (pdfium, `dpi`) →
/// uncompressed TIFF → tesseract TSV → blocks. Per-page failures and
/// timeouts warn and continue; OCR never fails the document.
pub fn ocr_pdf(
    pdf_path: &str,
    pages: &[u32],
    availability: &OcrAvailability,
    dpi: f32,
    page_timeout: Duration,
) -> CoreResult<OcrOutcome> {
    let binary = availability
        .binary
        .clone()
        .ok_or_else(|| CoreError::Ocr("tesseract binary not found".into()))?;

    let bytes = std::fs::read(pdf_path)
        .map_err(|e| CoreError::Ocr(format!("cannot read {pdf_path}: {e}")))?;
    let (pdfium, _) = crate::extract::pdf::bind_pdfium()?;
    let document = crate::extract::pdf::open_pdf(&pdfium, &bytes)?;

    let langs = availability.lang_arg();
    let mut outcome = OcrOutcome {
        blocks: Vec::new(),
        pages_done: Vec::new(),
        warnings: Vec::new(),
        mean_conf: 0.0,
    };
    let mut total_conf = 0.0f32;
    let mut total_words = 0usize;

    for page_no in pages {
        let page = match document.pages().get(page_no.saturating_sub(1) as i32) {
            Ok(p) => p,
            Err(e) => {
                outcome.warnings.push(super::labels::page_failed(
                    *page_no,
                    &format!("unreadable page: {e}"),
                ));
                continue;
            }
        };
        let raster: RasterizedPage = match render_page(&page, dpi) {
            Ok(r) => r,
            Err(e) => {
                outcome
                    .warnings
                    .push(super::labels::page_failed(*page_no, &e.to_string()));
                continue;
            }
        };
        let tiff = super::rasterize::encode_tiff_rgb(&raster);
        let image_path = match write_temp_tiff(&tiff) {
            Some(p) => p,
            None => {
                outcome.warnings.push(super::labels::page_failed(
                    *page_no,
                    "temp file creation failed",
                ));
                continue;
            }
        };
        let result = run_tesseract_image(&binary, &langs, &image_path, page_timeout);
        let _ = std::fs::remove_file(&image_path);
        let words = match result {
            Ok(w) => w,
            Err(e) => {
                outcome
                    .warnings
                    .push(super::labels::page_failed(*page_no, &e.to_string()));
                continue;
            }
        };
        if words.is_empty() {
            outcome.warnings.push(super::labels::page_failed(
                *page_no,
                "no text recognized (empty OCR result)",
            ));
            continue;
        }
        total_conf += words.iter().map(|w| w.conf).sum::<f32>();
        total_words += words.len();
        outcome.blocks.extend(words_to_blocks(
            &words,
            *page_no,
            raster.height_px as f32,
            raster.dpi,
        ));
        outcome.pages_done.push(*page_no);
    }

    if total_words > 0 {
        outcome.mean_conf = total_conf / total_words as f32;
    }
    Ok(outcome)
}

/// Run `tesseract <image> stdout -l <langs> --psm 3 tsv` on one image and
/// parse the words. Public for the integration test that OCRs a committed
/// fixture image directly.
pub fn run_tesseract_image(
    binary: &Path,
    langs: &str,
    image: &Path,
    timeout: Duration,
) -> CoreResult<Vec<OcrWord>> {
    let mut cmd = std::process::Command::new(binary);
    // Fixed argument shapes only — no user text ever reaches argv.
    cmd.arg(image)
        .arg("stdout")
        .arg("-l")
        .arg(langs)
        .arg("--psm")
        .arg("3")
        .arg("tsv");
    let out = run_with_timeout(&mut cmd, timeout)?;
    if out.timed_out {
        return Err(CoreError::Ocr(format!(
            "tesseract timed out after {}s",
            timeout.as_secs().max(1)
        )));
    }
    if !out.status_success {
        let stderr = out.stderr.lines().next_back().unwrap_or_default();
        return Err(CoreError::Ocr(format!(
            "tesseract exited with an error: {}",
            stderr.trim()
        )));
    }
    let tsv = String::from_utf8_lossy(&out.stdout);
    Ok(parse_tsv(&tsv))
}

/// Write TIFF bytes to a fresh temp file; `None` when the temp dir is
/// unwritable (callers warn and skip the page instead of failing).
fn write_temp_tiff(bytes: &[u8]) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("difflens-ocr");
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!(
        "page-{}-{}.tiff",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos()
    );
    let path = dir.join(name);
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture TSV snippet (as tesseract 5 emits): header, a page row,
    /// two lines, plus a malformed row and a non-word row to be skipped.
    const SAMPLE_TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t600\t200\t-1\t\n\
5\t1\t1\t1\t1\t1\t10\t10\t80\t20\t96.5\tHợp\n\
5\t1\t1\t1\t1\t2\t100\t10\t120\t20\t93.1\tđồng\n\
not-a-number\trow\n\
5\t1\t1\t1\t2\t1\t10\t60\t150\t20\t88.0\tThanh\n\
5\t1\t1\t1\t2\t2\t170\t60\t110\t20\t90.3\ttoán\n";

    #[test]
    fn parses_word_rows_and_skips_malformed() {
        let words = parse_tsv(SAMPLE_TSV);
        assert_eq!(words.len(), 4);
        assert_eq!(words[0].text, "Hợp");
        assert_eq!(words[0].left, 10);
        assert_eq!(words[0].conf, 96.5);
        assert_eq!(words[3].text, "toán");
    }

    #[test]
    fn skips_negative_confidence_and_short_rows() {
        let tsv = "5\t1\t1\t1\t1\t1\t10\t10\t80\t20\t-1\t\n\
5\t1\t1\t1\t1\t2\t5\t5\t10\n\
5\t1\t1\t1\t1\t3\t10\t10\t80\t20\t70\tkept\n";
        let words = parse_tsv(tsv);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "kept");
    }

    #[test]
    fn tolerates_empty_and_garbage_input() {
        assert!(parse_tsv("").is_empty());
        assert!(parse_tsv("\n\n\n").is_empty());
        assert!(parse_tsv("garbage\nrows\there").is_empty());
    }

    #[test]
    fn lines_cluster_by_bottom_and_order_by_left() {
        let words = parse_tsv(SAMPLE_TSV);
        let lines = words_to_lines(&words);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hợp đồng");
        assert_eq!(lines[1].text, "Thanh toán");
        assert_eq!(lines[0].top, 10);
        assert_eq!(lines[1].top, 60);
    }

    #[test]
    fn close_lines_merge_into_one_paragraph_block() {
        // Second line 50px below the first with 20px median height: the
        // union rect grows (top 10, height 70), so the next line gap check
        // uses the accumulated rect. First merge: 50 <= 1.6 * 20 = 32? No —
        // 50 > 32, so these two lines stay separate paragraphs. Verify that
        // split behavior here.
        let words = parse_tsv(SAMPLE_TSV);
        let blocks = words_to_blocks(&words, 3, 200.0, 300.0);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "Hợp đồng");
        assert_eq!(blocks[1].text, "Thanh toán");
        assert_eq!(blocks[0].page, Some(3));
        assert_eq!(blocks[0].anchor.as_deref(), Some("ocr"));
        let rect = blocks[0].rect.unwrap();
        // Pixel rects scaled back to points (300 dpi) and y-up origin.
        assert!((rect.x - 10.0 * 72.0 / 300.0).abs() < 1e-4);
        let expected_y = (200.0 - 30.0) * 72.0 / 300.0; // page_h - (top+h)
        assert!((rect.y - expected_y).abs() < 1e-4);
        assert!((rect.height - 20.0 * 72.0 / 300.0).abs() < 1e-4);
    }

    #[test]
    fn close_baselines_merge_into_one_paragraph() {
        // 20px gap between line tops with 20px heights: 20 <= 32 -> merged.
        let tsv = "5\t1\t1\t1\t1\t1\t10\t10\t80\t20\t90\tone\n\
5\t1\t1\t1\t2\t1\t12\t30\t80\t20\t90\ttwo\n";
        let words = parse_tsv(tsv);
        let blocks = words_to_blocks(&words, 1, 200.0, 300.0);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "one two");
        let rect = blocks[0].rect.unwrap();
        assert!(
            (rect.height - 40.0 * 72.0 / 300.0).abs() < 1e-4,
            "rect spans both lines"
        );
    }

    #[test]
    fn interleaved_insertion_order_does_not_break_reading_order() {
        let tsv = "5\t1\t1\t1\t2\t1\t10\t120\t80\t20\t90\tline2\n\
5\t1\t1\t1\t1\t1\t10\t10\t80\t20\t90\tline1\n\
5\t1\t1\t1\t3\t1\t10\t300\t80\t20\t90\tline3\n";
        let words = parse_tsv(tsv);
        let blocks = words_to_blocks(&words, 1, 500.0, 300.0);
        let text: Vec<&str> = blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(text, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn normalize_is_applied_to_ocr_text() {
        // extract::normalize collapses whitespace/NFC-normalizes; OCR text
        // goes through the same path (NBSP inside a recognized word).
        let tsv = "5\t1\t1\t1\t1\t1\t10\t10\t80\t20\t90\tdouble\u{00a0}space\n";
        let words = parse_tsv(tsv);
        let blocks = words_to_blocks(&words, 1, 100.0, 300.0);
        assert_eq!(blocks[0].text, "double space");
    }

    #[test]
    fn confidences_survive_parsing() {
        let words = parse_tsv(SAMPLE_TSV);
        let total: f32 = words.iter().map(|w| w.conf).sum();
        let mean = total / words.len() as f32;
        assert!((mean - 91.975).abs() < 1e-3);
    }
}
