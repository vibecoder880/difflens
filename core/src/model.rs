//! Shared data model for the whole DiffLens pipeline.
//!
//! Pipeline: `DocumentModel` (extract) → alignment (align) → `DiffResult`
//! (diff) → heuristic enrichment (heuristic) → report/persistence.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Supported document formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocFormat {
    Pdf,
    Docx,
    Txt,
    Xlsx,
    Pptx,
}

impl DocFormat {
    /// Guess the format from a file extension.
    pub fn from_extension(ext: &str) -> Option<DocFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "pdf" => Some(DocFormat::Pdf),
            "docx" => Some(DocFormat::Docx),
            "txt" | "md" => Some(DocFormat::Txt),
            "xlsx" | "xlsm" => Some(DocFormat::Xlsx),
            "pptx" => Some(DocFormat::Pptx),
            _ => None,
        }
    }
}

/// A rectangular region on a page/slide, in document points or pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Kind of an extracted content block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockKind {
    /// Body paragraph.
    Paragraph,
    /// Heading with its outline level (1-based).
    Heading { level: u8 },
    /// List item (ordered flag).
    ListItem { ordered: bool },
    /// Table with cell text, row-major.
    Table { rows: usize, cols: usize },
    /// A spreadsheet cell.
    Cell { sheet: String, row: u32, col: u32 },
    /// A presentation slide (0-based index within deck).
    Slide { index: u32 },
    /// Caption / footnote / header / footer.
    Caption,
}

/// One extracted content block: the atomic unit of diffing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Stable id: `<source-letter><index>` (e.g. `a12`), assigned per document.
    pub id: String,
    pub kind: BlockKind,
    /// Plain text content (for tables: joined cell text).
    pub text: String,
    /// 1-based page / slide number when the format has pages.
    pub page: Option<u32>,
    /// Bounding box on the page when known.
    pub rect: Option<Rect>,
    /// Format-specific extras (e.g. clause number heading text).
    pub anchor: Option<String>,
}

/// An extracted document: ordered content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentModel {
    pub path: String,
    pub format: DocFormat,
    pub blocks: Vec<Block>,
    /// Pages / slides count when applicable.
    pub page_count: Option<u32>,
    /// Extraction warnings that users should see (non-fatal).
    pub warnings: Vec<String>,
}

impl DocumentModel {
    pub fn format_of(path: &str) -> CoreResult<DocFormat> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| CoreError::Extraction("file has no extension".into()))?;
        DocFormat::from_extension(ext)
            .ok_or_else(|| CoreError::Extraction(format!("unsupported extension: {ext}")))
    }
}

/// Diff operation for a text span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanOp {
    Equal,
    Insert,
    Delete,
}

/// A piece of inline text diff between two aligned blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSpan {
    pub op: SpanOp,
    pub text: String,
}

/// How a pair (or group) of blocks changed between versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    /// Block exists only in the new document.
    Added,
    /// Block exists only in the old document.
    Removed,
    /// Block was matched across documents and text differs.
    Modified,
    /// Same content detected at a different position.
    Moved,
    /// Identical content (kept for complete pair mapping).
    Equal,
}

/// Deterministic severity band, computed from rules only (no LLM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// One fired heuristic rule, with the evidence trail for audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeuristicFlag {
    /// Stable rule id, e.g. `MONEY_CHANGED`, `DATE_CHANGED`, `TERM_SHORTENED`.
    pub rule_id: String,
    /// Human-readable label key (i18n lookup), e.g. `flag.moneyChanged`.
    pub label_key: String,
    /// Raw value found in the old text, if any.
    pub old_value: Option<String>,
    /// Raw value found in the new text, if any.
    pub new_value: Option<String>,
    /// Normalized numeric delta when computable (e.g. -0.5 = −50 %).
    pub delta: Option<f64>,
    /// Extra context (unit, rule params).
    pub detail: Option<String>,
}

/// A diff entry: aligned old/new blocks plus inline spans and flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    /// Stable change id (`c0`, `c1`, ...).
    pub id: String,
    pub kind: ChangeKind,
    /// Block ids in the old document (empty for pure additions).
    pub old_block_ids: Vec<String>,
    /// Block ids in the new document (empty for pure removals).
    pub new_block_ids: Vec<String>,
    /// Old page (1-based) when known.
    pub old_page: Option<u32>,
    /// New page (1-based) when known.
    pub new_page: Option<u32>,
    /// Inline word-level diff (present for Modified/Moved pairs).
    pub spans: Vec<DiffSpan>,
    /// Cosine similarity of the aligned pair (0..1), when computed.
    pub similarity: Option<f32>,
    /// Position delta in points for moved blocks (dx, dy).
    pub move_delta: Option<(f32, f32)>,
    /// Deterministic severity (set by heuristics; None until enriched).
    pub severity: Option<Severity>,
    /// All heuristic rules that fired for this change (audit trail).
    pub flags: Vec<HeuristicFlag>,
}

/// A diff of a pair of documents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub old_path: String,
    pub new_path: String,
    pub format: DocFormat,
    pub changes: Vec<Change>,
    /// Summary counts by kind.
    pub stats: DiffStats,
    /// Whether real (ort) embeddings were used for alignment.
    pub embeddings_used: bool,
    /// Heuristic audit trail: one entry per enriched (non-Equal) change,
    /// in change order. Empty until the heuristic stage runs.
    pub audit: Vec<AuditEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffStats {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub moved: usize,
    pub equal: usize,
}

/// Which heuristic severity band a change landed in, plus the rule evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub change_id: String,
    pub severity: Severity,
    /// Ordered rule firings with reasons — the audit trail.
    pub rules: Vec<HeuristicFlag>,
}
