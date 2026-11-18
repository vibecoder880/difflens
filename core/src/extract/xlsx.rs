//! XLSX extraction via calamine.
//!
//! calamine reads cached cell values and formula strings but never evaluates
//! formulas — exactly what the diff needs ("formula changed, cached value
//! same" is decidable in Phase 3). Per visible sheet we emit one
//! [`BlockKind::Cell`] block per non-empty cell; fully empty rows are
//! skipped; row counts are capped with a warning.
//!
//! Note: calamine decompresses internally, so the zip-bomb budget of
//! [`super::ZipBudget`] does not apply here; the row cap is the resource
//! guard for this format.

use crate::error::{CoreError, CoreResult};
use crate::model::{Block, BlockKind, DocFormat, DocumentModel};

use super::normalize;
use super::{ExtractedParts, MAX_XLSX_ROWS, read_file};
use calamine::{Data, Reader, SheetVisible, Xlsx};
use std::io::Cursor;

/// Extract an XLSX from disk.
pub fn extract_xlsx(path: &str) -> CoreResult<DocumentModel> {
    let bytes = read_file(path)?;
    let parts = extract_xlsx_bytes(path, &bytes)?;
    Ok(super::build_model(path, DocFormat::Xlsx, parts))
}

pub(crate) fn extract_xlsx_bytes(_path: &str, bytes: &[u8]) -> CoreResult<ExtractedParts> {
    let mut workbook: Xlsx<Cursor<&[u8]>> = Xlsx::new(Cursor::new(bytes))
        .map_err(|e| CoreError::Extraction(format!("invalid xlsx: {e}")))?;

    let mut blocks: Vec<Block> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let visible_sheets: Vec<String> = workbook
        .sheets_metadata()
        .iter()
        .filter(|s| s.visible == SheetVisible::Visible)
        .map(|s| s.name.clone())
        .collect();
    let hidden = workbook.sheets_metadata().len() - visible_sheets.len();
    if hidden > 0 {
        warnings.push(format!("skipped {hidden} hidden sheet(s)"));
    }

    for sheet_name in visible_sheets {
        let range = match workbook.worksheet_range(&sheet_name) {
            Ok(r) => r,
            Err(e) => {
                warnings.push(format!(
                    "sheet {sheet_name:?} is unreadable and was skipped: {e}"
                ));
                continue;
            }
        };
        let formulas = workbook.worksheet_formula(&sheet_name).ok();
        let (start_row, start_col) = range.start().unwrap_or((0, 0));
        let (start_row, start_col) = (start_row as usize, start_col as usize);

        let mut emitted_rows = 0usize;
        let mut truncated = false;
        for (row_offset, row) in range.rows().enumerate() {
            let row_idx = start_row + row_offset;
            if emitted_rows >= MAX_XLSX_ROWS {
                truncated = true;
                break;
            }

            // Find the last non-empty column so trailing blanks don't widen
            // the block list.
            let has_content = |d: &Data| match d {
                Data::Empty => false,
                Data::String(s) => !s.trim().is_empty(),
                _ => true,
            };
            let Some(last_cell) = row.iter().rposition(has_content) else {
                continue;
            };

            let mut row_has_content = false;
            for (col_offset, data) in row.iter().take(last_cell + 1).enumerate() {
                let col_idx = start_col + col_offset;
                let value_text = cell_text(data);
                let formula_text = formulas
                    .as_ref()
                    .and_then(|f| f.get_value((row_idx as u32, col_idx as u32)))
                    .map(|f| f.trim().to_string())
                    .filter(|f| !f.is_empty());

                let is_formula = formula_text.is_some();
                let text = normalize::normalize(&formula_text.unwrap_or(value_text));
                if text.is_empty() {
                    continue;
                }
                row_has_content = true;
                blocks.push(Block {
                    id: String::new(),
                    kind: BlockKind::Cell {
                        sheet: sheet_name.clone(),
                        row: row_idx as u32,
                        col: col_idx as u32,
                    },
                    text,
                    page: None,
                    rect: None,
                    // Formula cells remember that their text is a formula
                    // string, not a cached value.
                    anchor: if is_formula {
                        Some("formula".to_string())
                    } else {
                        None
                    },
                });
            }
            if row_has_content {
                emitted_rows += 1;
            }
        }
        if truncated {
            warnings.push(format!(
                "sheet {sheet_name:?} exceeds {MAX_XLSX_ROWS} rows; truncated"
            ));
        }
    }

    Ok(ExtractedParts {
        blocks,
        page_count: None,
        warnings,
    })
}

/// Render a cached cell value deterministically. calamine's `Display` already
/// prints floats Excel-style (1.0 → "1", 1.5 → "1.5") and errors as
/// "#DIV/0!"-style strings, so we delegate.
fn cell_text(data: &Data) -> String {
    match data {
        Data::Empty => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_text_is_deterministic() {
        assert_eq!(cell_text(&Data::Float(1.0)), "1");
        assert_eq!(cell_text(&Data::Float(1.5)), "1.5");
        assert_eq!(cell_text(&Data::String("héllo".into())), "héllo");
        assert_eq!(cell_text(&Data::Bool(true)), "true");
        assert_eq!(cell_text(&Data::Empty), "");
    }
}
