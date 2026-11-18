#!/usr/bin/env node
// Generate the phase-06 OCR fixtures:
//   core/tests/fixtures/vietnamese_scanned.png  — Vietnamese contract text raster
//   core/tests/fixtures/vietnamese_scanned.pdf  — image-only ("scanned") PDF
//                                                 wrapping the same raster as a
//                                                 full-page FlateDecode RGB image
//
// The committed fixtures are the source of truth for CI determinism (CI never
// runs this script). Regenerate ONLY when the fixture text must change:
//
//   npm i --prefix /tmp/ocrfixgen @napi-rs/canvas
//   node scripts/gen-fixtures/gen_ocr_fixtures.mjs
//   (override the module dir with OCR_CANVAS_MODULES=<dir>/node_modules)
//
// Why @napi-rs/canvas: no PIL/ImageMagick/rsvg on the dev VM; @napi-rs/canvas
// is a prebuilt Skia binary that shapes Vietnamese diacritics correctly with
// the installed DejaVuSans.ttf (fc-list confirms Vietnamese coverage).
//
// Determinism: fixed canvas size, font size and positions; PNG encoding and
// zlib are deterministic per toolchain version. The PDF embeds the exact raw
// RGBA pixels (converted to RGB) compressed with zlib — no timestamps.

import { createRequire } from "node:module";
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const modulesDir =
  process.env.OCR_CANVAS_MODULES ?? "/tmp/ocrfixgen/node_modules";
const { createCanvas, GlobalFonts } = createRequire(
  join(modulesDir, "noop.js"),
)("@napi-rs/canvas");

const FONT_PATH = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
const WIDTH = 1000;
const HEIGHT = 420;
// 300 DPI canvas raster -> PDF page size in points (1 pt = 1/72 inch).
const PAGE_W = (WIDTH / 300) * 72;
const PAGE_H = (HEIGHT / 300) * 72;

// Common Vietnamese contract vocabulary (phase contract words). Tesseract vie
// must recover at least "Thanh toán" and the diacritic set from these.
const LINES = [
  "Hợp đồng bảo hiểm số 2026-VN-0142",
  "Bên A thanh toán cho Bên B trong 30 ngày",
  "Bồi thường thiệt hại theo hợp đồng đã ký",
];

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const outDir = join(repoRoot, "core", "tests", "fixtures");

GlobalFonts.registerFromPath(FONT_PATH, "DejaVu");
const canvas = createCanvas(WIDTH, HEIGHT);
const ctx = canvas.getContext("2d");

ctx.fillStyle = "#ffffff";
ctx.fillRect(0, 0, WIDTH, HEIGHT);
ctx.fillStyle = "#000000";
ctx.font = "40px DejaVu";
ctx.textBaseline = "alphabetic";
LINES.forEach((line, i) => {
  ctx.fillText(line, 50, 120 + i * 100);
});

// 1) PNG fixture (committed; CI and the engine-level test use it directly).
const png = canvas.toBuffer("image/png");
const pngPath = join(outDir, "vietnamese_scanned.png");
writeFileSync(pngPath, png);

// 2) Image-only PDF embedding the same raster as raw RGB (FlateDecode).
const rgba = ctx.getImageData(0, 0, WIDTH, HEIGHT).data;
const rgb = Buffer.alloc(WIDTH * HEIGHT * 3);
for (let i = 0, j = 0; i < rgba.length; i += 4, j += 3) {
  rgb[j] = rgba[i];
  rgb[j + 1] = rgba[i + 1];
  rgb[j + 2] = rgba[i + 2];
}
const imageStream = deflateSync(rgb, { level: 9 });

const content = `q ${PAGE_W.toFixed(2)} 0 0 ${PAGE_H.toFixed(2)} 0 0 cm /Im0 Do Q\n`;
const objects = [
  "<< /Type /Catalog /Pages 2 0 R >>",
  "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
  `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${PAGE_W.toFixed(2)} ${PAGE_H.toFixed(
    2,
  )}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>`,
  `<< /Type /XObject /Subtype /Image /Width ${WIDTH} /Height ${HEIGHT} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length ${imageStream.length} >>\nstream\n`,
  `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}endstream\nendobj\n`,
];

let pdf = "%PDF-1.4\n";
const offsets = [];
objects.forEach((body, i) => {
  offsets.push(Buffer.byteLength(pdf, "latin1"));
  pdf += `${i + 1} 0 obj\n${body}`;
  if (i === 3) {
    // image object: body ends before `stream\n`, append data + endstream
    pdf += `${imageStream.toString("latin1")}endstream\nendobj\n`;
  } else if (i === 4) {
    pdf += `${content}endstream\nendobj\n`;
  } else {
    pdf += `endobj\n`;
  }
});
const xrefOffset = Buffer.byteLength(pdf, "latin1");
pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
for (const off of offsets) {
  pdf += `${String(off).padStart(10, "0")} 00000 n \n`;
}
pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;

const pdfPath = join(outDir, "vietnamese_scanned.pdf");
writeFileSync(pdfPath, Buffer.from(pdf, "latin1"));

console.log(`wrote ${pngPath} (${png.length} bytes)`);
console.log(`wrote ${pdfPath} (${Buffer.byteLength(pdf, "latin1")} bytes)`);
