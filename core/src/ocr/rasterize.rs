//! Rasterize PDF pages to bitmaps for the OCR path (phase 06).
//!
//! Uses the Phase-02 runtime-bound pdfium loader (no new dependencies):
//! each page is rendered into a BGRA bitmap at the configured DPI, reduced
//! to plain RGB, then encoded as an **uncompressed little-endian TIFF**
//! (classic TIFF, single strip) — the format Tesseract/Leptonica reads most
//! reliably.
//!
//! # Why a hand-rolled TIFF writer instead of the `image` crate
//!
//! Uncompressed classic TIFF is ~70 lines with zero dependencies, fully
//! deterministic byte-for-byte (no compressor tuning, no timestamps — the
//! DateTime tag is deliberately omitted), and lossless by construction.
//! Pulling in `image` (or `tiff`) would add several codecs this project
//! never uses, for no determinism gain. The writer is round-trip verified
//! by a unit test that parses the produced header back.
//!
//! Deterministic output: same page bytes + same DPI ⇒ same TIFF bytes.
//! pdfium-render is used with `default-features = false` (as in Phase 02);
//! raw-buffer rendering needs no image-format features.

use pdfium_render::prelude::*;

use crate::error::{CoreError, CoreResult};

/// One rasterized page: packed RGB rows (`width_px * 3` bytes per row, no
/// padding), in top-down pixel order.
pub struct RasterizedPage {
    pub width_px: u32,
    pub height_px: u32,
    /// Render DPI used (bbox coordinates scale back to PDF points with it).
    pub dpi: f32,
    pub rgb: Vec<u8>,
}

/// Render one PDF page into an RGB bitmap at `dpi` (target 300, per phase
/// contract; configurable via [`super::OcrOptions`]).
pub(crate) fn render_page(page: &PdfPage, dpi: f32) -> CoreResult<RasterizedPage> {
    if !dpi.is_finite() || dpi < 36.0 || dpi > 600.0 {
        return Err(CoreError::Ocr(format!(
            "rasterize dpi {dpi} outside the supported 36..=600 range"
        )));
    }
    let scale = dpi / 72.0;
    let width_px = ((page.width().value * scale).round() as u32).max(1);
    let height_px = ((page.height().value * scale).round() as u32).max(1);

    let config = PdfRenderConfig::new()
        .set_target_width(width_px as Pixels)
        .set_target_height(height_px as Pixels)
        .set_format(PdfBitmapFormat::BGRA);
    let bitmap = page
        .render_with_config(&config)
        .map_err(|e| CoreError::Ocr(format!("pdfium render failed: {e}")))?;

    let rendered_w = bitmap.width() as u32;
    let rendered_h = bitmap.height() as u32;
    let raw = bitmap.as_raw_bytes();
    // BGRA pixels: stride is a multiple of 4 (== width * 4 for BGRA), but we
    // never assume it — the stride is derived from the buffer length.
    let stride = if rendered_h == 0 {
        0
    } else {
        raw.len() / rendered_h as usize
    };

    let mut rgb = Vec::with_capacity((rendered_w * rendered_h * 3) as usize);
    for row in 0..rendered_h as usize {
        let row_start = row * stride;
        for px in 0..rendered_w as usize {
            let base = row_start + px * 4;
            if base + 3 >= raw.len() {
                return Err(CoreError::Ocr("pdfium bitmap buffer truncated".into()));
            }
            // BGRA -> RGB, drop alpha (pdfium guarantees an opaque buffer).
            rgb.push(raw[base + 2]);
            rgb.push(raw[base + 1]);
            rgb.push(raw[base]);
        }
    }

    Ok(RasterizedPage {
        width_px: rendered_w,
        height_px: rendered_h,
        dpi,
        rgb,
    })
}

// ---------------------------------------------------------------------------
// Uncompressed TIFF encoder
// ---------------------------------------------------------------------------

const TIFF_TAG_IMAGE_WIDTH: u16 = 256;
const TIFF_TAG_IMAGE_LENGTH: u16 = 257;
const TIFF_TAG_BITS_PER_SAMPLE: u16 = 258;
const TIFF_TAG_COMPRESSION: u16 = 259;
const TIFF_TAG_PHOTOMETRIC: u16 = 262;
const TIFF_TAG_STRIP_OFFSETS: u16 = 273;
const TIFF_TAG_SAMPLES_PER_PIXEL: u16 = 277;
const TIFF_TAG_ROWS_PER_STRIP: u16 = 278;
const TIFF_TAG_STRIP_BYTE_COUNTS: u16 = 279;
const TIFF_TAG_X_RESOLUTION: u16 = 282;
const TIFF_TAG_Y_RESOLUTION: u16 = 283;
const TIFF_TAG_RESOLUTION_UNIT: u16 = 296;

const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_RATIONAL: u16 = 5;

/// Encode an [`RasterizedPage`] as an uncompressed RGB classic TIFF
/// (little-endian, single strip, no DateTime tag → deterministic bytes).
pub fn encode_tiff_rgb(page: &RasterizedPage) -> Vec<u8> {
    let width = page.width_px;
    let height = page.height_px;
    let data_len = (width as usize) * (height as usize) * 3;

    let mut out = Vec::with_capacity(180 + data_len);
    // Header: little-endian ("II"), magic 42, IFD at offset 8.
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes());

    // Extra-value blobs follow the IFD. Entries are emitted in ascending tag
    // order (a TIFF requirement).
    let entries: [(u16, u16, u32, [u8; 4], bool); 12] = [
        (
            TIFF_TAG_IMAGE_WIDTH,
            TYPE_LONG,
            1,
            width.to_le_bytes(),
            false,
        ),
        (
            TIFF_TAG_IMAGE_LENGTH,
            TYPE_LONG,
            1,
            height.to_le_bytes(),
            false,
        ),
        // 3 SHORTs (6 bytes) do not fit the inline 4-byte value field.
        (TIFF_TAG_BITS_PER_SAMPLE, TYPE_SHORT, 3, [0; 4], true),
        (TIFF_TAG_COMPRESSION, TYPE_SHORT, 1, [1, 0, 0, 0], false),
        (TIFF_TAG_PHOTOMETRIC, TYPE_SHORT, 1, [2, 0, 0, 0], false), // RGB
        (TIFF_TAG_STRIP_OFFSETS, TYPE_LONG, 1, [0; 4], false),      // patched below
        (
            TIFF_TAG_SAMPLES_PER_PIXEL,
            TYPE_SHORT,
            1,
            [3, 0, 0, 0],
            false,
        ),
        (
            TIFF_TAG_ROWS_PER_STRIP,
            TYPE_LONG,
            1,
            height.to_le_bytes(),
            false,
        ),
        (
            TIFF_TAG_STRIP_BYTE_COUNTS,
            TYPE_LONG,
            1,
            (data_len as u32).to_le_bytes(),
            false,
        ),
        (TIFF_TAG_X_RESOLUTION, TYPE_RATIONAL, 1, [0; 4], true),
        (TIFF_TAG_Y_RESOLUTION, TYPE_RATIONAL, 1, [0; 4], true),
        (TIFF_TAG_RESOLUTION_UNIT, TYPE_SHORT, 1, [2, 0, 0, 0], false), // inch
    ];

    let ifd_offset = 8u32;
    let ifd_len = 2 + (entries.len() as u32) * 12 + 4;
    let mut extra_offset = ifd_offset + ifd_len;

    // Locate where the strip data will start; extra blobs are laid out in
    // entry order right after the IFD.
    let mut bits_offset = 0u32;
    let mut xres_offset = 0u32;
    let mut yres_offset = 0u32;
    for (tag, _, _, _, has_extra) in entries {
        if has_extra {
            match tag {
                TIFF_TAG_BITS_PER_SAMPLE => bits_offset = extra_offset,
                TIFF_TAG_X_RESOLUTION => xres_offset = extra_offset,
                TIFF_TAG_Y_RESOLUTION => yres_offset = extra_offset,
                _ => {}
            }
            extra_offset += match tag {
                TIFF_TAG_BITS_PER_SAMPLE => 6,
                _ => 8, // RATIONAL
            };
        }
    }
    let strip_offset = extra_offset as usize;

    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, typ, count, value, has_extra) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&typ.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if has_extra {
            let at = match tag {
                TIFF_TAG_BITS_PER_SAMPLE => bits_offset,
                TIFF_TAG_X_RESOLUTION => xres_offset,
                _ => yres_offset,
            };
            out.extend_from_slice(&at.to_le_bytes());
        } else if tag == TIFF_TAG_STRIP_OFFSETS {
            out.extend_from_slice(&(strip_offset as u32).to_le_bytes());
        } else {
            out.extend_from_slice(&value);
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // no next IFD

    // Extra blobs, in the same order the offsets were assigned.
    out.extend_from_slice(&8u16.to_le_bytes()); // bits per sample: 8,8,8
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    let dpi_fixed = (page.dpi.max(1.0) * 100.0).round() as u32;
    out.extend_from_slice(&dpi_fixed.to_le_bytes()); // X: dpi*100 / 100
    out.extend_from_slice(&100u32.to_le_bytes());
    out.extend_from_slice(&dpi_fixed.to_le_bytes()); // Y
    out.extend_from_slice(&100u32.to_le_bytes());

    debug_assert_eq!(out.len(), strip_offset);
    out.extend_from_slice(&page.rgb[..data_len.min(page.rgb.len())]);
    while out.len() < strip_offset + data_len {
        out.push(0); // pad deterministically on short input (never in practice)
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_page(w: u32, h: u32) -> RasterizedPage {
        RasterizedPage {
            width_px: w,
            height_px: h,
            dpi: 300.0,
            rgb: vec![255u8; (w * h * 3) as usize],
        }
    }

    /// Minimal parser for the exact classic-TIFF shape this module writes.
    fn parse_ifd(bytes: &[u8]) -> Vec<(u16, u16, u32, u32)> {
        assert_eq!(&bytes[0..2], b"II", "little-endian marker");
        assert_eq!(u16::from_le_bytes(bytes[2..4].try_into().unwrap()), 42);
        let ifd = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let count =
            u16::from_le_bytes(bytes[ifd as usize..ifd as usize + 2].try_into().unwrap()) as usize;
        let mut out = Vec::new();
        for i in 0..count {
            let at = ifd as usize + 2 + i * 12;
            let tag = u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
            let typ = u16::from_le_bytes(bytes[at + 2..at + 4].try_into().unwrap());
            let n = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
            let value = u32::from_le_bytes(bytes[at + 8..at + 12].try_into().unwrap());
            out.push((tag, typ, n, value));
        }
        out
    }

    #[test]
    fn tiff_header_parses_back_with_expected_tags() {
        let page = solid_page(8, 6);
        let bytes = encode_tiff_rgb(&page);
        let entries = parse_ifd(&bytes);
        let find = |tag: u16| entries.iter().copied().find(|e| e.0 == tag).unwrap();
        assert_eq!(find(256), (256, 4, 1, 8)); // width
        assert_eq!(find(257), (257, 4, 1, 6)); // height
        assert_eq!(find(258).2, 3); // bits per sample count == 3
        assert_eq!(find(259), (259, 3, 1, 1)); // compression: none
        assert_eq!(find(262), (262, 3, 1, 2)); // photometric: RGB
        assert_eq!(find(277), (277, 3, 1, 3)); // samples per pixel
        assert_eq!(find(278), (278, 4, 1, 6)); // rows per strip == height
        assert_eq!(find(279), (279, 4, 1, 8 * 6 * 3)); // strip byte counts
        assert_eq!(find(296), (296, 3, 1, 2)); // unit: inch
        // Strip offsets point past the extra-value blobs.
        let strip = find(273).3 as usize;
        assert!(strip < bytes.len());
        assert_eq!(bytes.len() - strip, 8 * 6 * 3);
    }

    #[test]
    fn tiff_is_deterministic() {
        let a = encode_tiff_rgb(&solid_page(16, 9));
        let b = encode_tiff_rgb(&solid_page(16, 9));
        assert_eq!(a, b);
    }

    #[test]
    fn tiff_carries_dpi_resolution() {
        let mut page = solid_page(4, 4);
        page.dpi = 300.0;
        let bytes = encode_tiff_rgb(&page);
        let xres = parse_ifd(&bytes)
            .iter()
            .copied()
            .find(|e| e.0 == 282)
            .unwrap()
            .3 as usize;
        let (num, den) = (
            u32::from_le_bytes(bytes[xres..xres + 4].try_into().unwrap()),
            u32::from_le_bytes(bytes[xres + 4..xres + 8].try_into().unwrap()),
        );
        assert_eq!((num, den), (30000, 100));
    }

    #[test]
    fn tiff_strips_never_padded_mid_row() {
        // A width whose *3 row size is not 4-aligned must still produce a
        // tight single-strip payload (rows packed without padding).
        let page = solid_page(5, 3);
        let bytes = encode_tiff_rgb(&page);
        let strip = parse_ifd(&bytes)
            .iter()
            .copied()
            .find(|e| e.0 == 273)
            .unwrap()
            .3 as usize;
        assert_eq!(bytes.len() - strip, 5 * 3 * 3);
    }
}
