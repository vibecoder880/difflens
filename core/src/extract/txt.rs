//! Plain-text extraction: BOM-aware decoding, paragraph splitting on blank
//! lines, markdown ATX headings (`#` … `######`).

use crate::error::{CoreError, CoreResult};
use crate::model::{Block, BlockKind, DocFormat, DocumentModel};

use super::normalize;
use super::{ExtractedParts, MAX_TXT_BYTES, read_file};

/// Extract a plain-text / markdown document from disk.
pub fn extract_txt(path: &str) -> CoreResult<DocumentModel> {
    let bytes = read_file(path)?;
    let parts = extract_txt_bytes(path, &bytes)?;
    Ok(super::build_model(path, DocFormat::Txt, parts))
}

pub(crate) fn extract_txt_bytes(_path: &str, bytes: &[u8]) -> CoreResult<ExtractedParts> {
    if bytes.len() > MAX_TXT_BYTES {
        return Err(CoreError::Extraction(format!(
            "text file exceeds the {MAX_TXT_BYTES} byte limit"
        )));
    }
    let (text, warnings) = decode(bytes);
    let blocks = split_blocks(&text);
    Ok(ExtractedParts {
        blocks,
        page_count: None,
        warnings,
    })
}

/// BOM-aware decoding. UTF-8 is the default; UTF-16 is detected via its BOM;
/// non-UTF-8 bytes without a BOM are decoded lossily with a warning
/// (deterministic: every invalid byte becomes U+FFFD).
fn decode(bytes: &[u8]) -> (String, Vec<String>) {
    let mut warnings = Vec::new();
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(&bytes[3..]).into_owned(), warnings);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let big_endian = bytes[0] == 0xFE;
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                if big_endian {
                    u16::from_be_bytes(*p)
                } else {
                    u16::from_le_bytes(*p)
                }
            })
            .collect();
        return (String::from_utf16_lossy(&units), warnings);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_owned(), warnings),
        Err(_) => {
            warnings.push("text is not valid UTF-8; decoded lossily".to_string());
            (String::from_utf8_lossy(bytes).into_owned(), warnings)
        }
    }
}

/// Split into blocks. A markdown heading line always forms its own block;
/// runs of non-blank lines between blank lines form paragraph blocks.
fn split_blocks(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();

    for line in text.lines() {
        if let Some(level) = markdown_heading_level(line) {
            flush_paragraph(&mut paragraph, &mut blocks);
            let title = line.trim_start_matches('#');
            push_block(&mut blocks, normalize::normalize(title), || {
                BlockKind::Heading { level }
            });
        } else if line.trim().is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
        } else {
            paragraph.push(line);
        }
    }
    flush_paragraph(&mut paragraph, &mut blocks);
    blocks
}

fn flush_paragraph(paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>) {
    if paragraph.is_empty() {
        return;
    }
    let raw = paragraph.join("\n");
    let text = normalize::normalize(&raw);
    if !text.is_empty() {
        blocks.push(Block {
            id: String::new(),
            kind: BlockKind::Paragraph,
            text,
            page: None,
            rect: None,
            anchor: None,
        });
    }
    paragraph.clear();
}

fn push_block<F>(blocks: &mut Vec<Block>, text: String, kind: F)
where
    F: FnOnce() -> BlockKind,
{
    if !text.is_empty() {
        blocks.push(Block {
            id: String::new(),
            kind: kind(),
            text,
            page: None,
            rect: None,
            anchor: None,
        });
    }
}

/// ATX heading level (1-6): 1-6 leading `#` characters followed by a space,
/// a tab or end-of-line.
fn markdown_heading_level(line: &str) -> Option<u8> {
    let trimmed = line.trim_start_matches(' ');
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
        Some(hashes as u8)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DocFormat;

    #[test]
    fn headings_and_paragraphs() {
        let text = "# Title\n\nIntro line one\ncontinues here\n\n## Sub\n\nBody\n";
        let blocks = split_blocks(text);
        let kinds: Vec<&BlockKind> = blocks.iter().map(|b| &b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &BlockKind::Heading { level: 1 },
                &BlockKind::Paragraph,
                &BlockKind::Heading { level: 2 },
                &BlockKind::Paragraph
            ]
        );
        assert_eq!(blocks[1].text, "Intro line one continues here");
        assert_eq!(blocks[2].text, "Sub");
    }

    #[test]
    fn hash_without_space_is_not_a_heading() {
        assert_eq!(markdown_heading_level("#tag"), None);
        assert_eq!(markdown_heading_level("####### seven"), None);
        assert_eq!(markdown_heading_level("## Space"), Some(2));
        assert_eq!(markdown_heading_level("###### Six"), Some(6));
        assert_eq!(markdown_heading_level("Plain # not"), None);
    }

    #[test]
    fn utf16le_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for u in "Xin chào".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let (text, warnings) = decode(&bytes);
        assert_eq!(text, "Xin chào");
        assert!(warnings.is_empty());
    }

    #[test]
    fn invalid_utf8_is_lossy_with_warning() {
        let (text, warnings) = decode(&[b'a', 0xFF, b'b']);
        assert_eq!(text, "a\u{FFFD}b");
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn format_is_txt() {
        assert_eq!(DocFormat::from_extension("md"), Some(DocFormat::Txt));
    }
}
