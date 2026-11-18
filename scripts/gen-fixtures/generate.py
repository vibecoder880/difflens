#!/usr/bin/env python3
"""Regenerate the binary fixtures committed under core/tests/fixtures/.

All fixtures are deterministic (fixed zip timestamps, fixed PDF object
layout) and tiny (<20 KB each). Text fixtures are committed directly.

Usage: python3 -I scripts/gen-fixtures/generate.py
"""

import os
import struct
import unicodedata
import zlib
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIXTURES = os.path.join(ROOT, "core", "tests", "fixtures")
ZIP_DATE = (2026, 1, 1, 0, 0, 0)


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    print("wrote", os.path.relpath(path, ROOT), len(data), "bytes")


def add_zip_entry(z, name, data):
    info = zipfile.ZipInfo(name, date_time=ZIP_DATE)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o600 << 16
    z.writestr(info, data)


def content_types(*overrides):
    defaults = (
        '<Default Extension="rels" ContentType='
        '"application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
    )
    body = "".join(
        f'<Override PartName="{part}" ContentType="{ctype}"/>'
        for part, ctype in overrides
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        + defaults + body + "</Types>"
    ).encode()


def rels(relationships):
    body = "".join(
        f'<Relationship Id="{rid}" Type="{rtype}" Target="{target}"/>'
        for rid, rtype, target in relationships
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + body + "</Relationships>"
    ).encode()


# --------------------------------------------------------------------------
# TXT fixtures (plain text, committed directly)
# --------------------------------------------------------------------------

def gen_txt():
    sample = (
        "# Report\n"
        "\n"
        "First paragraph with a short line.\n"
        "\n"
        "Second paragraph that continues\n"
        "on a second physical line.\n"
        "\n"
        "## Details\n"
        "\n"
        "Final body paragraph mentioning 100 USD and 2026-01-15.\n"
    )
    write(os.path.join(FIXTURES, "sample.txt"), sample.encode())
    write(os.path.join(FIXTURES, "empty.txt"), b"")

    # NFD (decomposed) Vietnamese so NFC normalization is exercised.
    nfc = "Xin chào\n\nĐây là văn bản tiếng Việt có dấu.\n"
    nfd = unicodedata.normalize("NFD", nfc).encode()
    assert nfd != nfc.encode()
    write(os.path.join(FIXTURES, "vietnamese.txt"), nfd)


# --------------------------------------------------------------------------
# PDF fixture (hand-written PDF bytes, 3 pages: two text pages + one blank)
# --------------------------------------------------------------------------

def pdf_escape(text):
    return text.replace("\\", r"\\").replace("(", r"\(").replace(")", r"\)")


def gen_pdf():
    # (font_size, x, y, text); y is measured from the page bottom (y-up).
    # Baseline distances: 8pt within a paragraph (merges at any plausible
    # median glyph height), >= 30pt between paragraphs (always splits).
    page1 = [
        (14, 72, 720, "DiffLens PDF Fixture"),
        (12, 72, 678, "This is the first body paragraph spanning"),
        (12, 72, 670, "two lines that must merge into one block."),
        (12, 72, 640, "A third paragraph after a wider gap."),
    ]
    page2 = [
        (12, 72, 700, "Second page text."),
    ]

    def content_stream(lines):
        ops = ["BT"]
        for size, x, y, text in lines:
            ops.append(f"/F1 {size} Tf 1 0 0 1 {x} {y} Tm")
            ops.append(f"({pdf_escape(text)}) Tj")
        ops.append("ET")
        return "\n".join(ops).encode()

    objects = []
    objects.append(b"<< /Type /Catalog /Pages 2 0 R >>")  # 1
    kids = "3 0 R 6 0 R 8 0 R"
    objects.append(f"<< /Type /Pages /Kids [{kids}] /Count 3 >>".encode())  # 2
    objects.append(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
    )  # 3 page 1
    objects.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")  # 4
    objects.append(b"<< /Length %d >>\nstream\n%b\nendstream" % (len(content_stream(page1)), content_stream(page1)))  # 5
    objects.append(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R >>"
    )  # 6 page 2
    objects.append(b"<< /Length %d >>\nstream\n%b\nendstream" % (len(content_stream(page2)), content_stream(page2)))  # 7
    # Page 8: intentionally blank (no text layer) → scanned-pdf warning path.
    objects.append(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>"
    )  # 8

    out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for i, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
    xref_start = len(out)
    out += b"xref\n0 %d\n" % (len(objects) + 1)
    out += b"0000000000 65535 f \n"
    for off in offsets:
        out += b"%010d 00000 n \n" % off
    out += (
        b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n"
        % (len(objects) + 1, xref_start)
    )
    write(os.path.join(FIXTURES, "sample.pdf"), bytes(out))


# --------------------------------------------------------------------------
# DOCX fixtures
# --------------------------------------------------------------------------

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
DOC_BODY_OPEN = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    f'<w:document xmlns:w="{W_NS}"><w:body>'
)


def wpara(inner, style=None):
    style_xml = f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else ""
    return f"<w:p>{style_xml}{inner}</w:p>"


def wrun(text, tab=False, br=False):
    inner = ""
    if tab:
        inner += "<w:tab/>"
    inner += f"<w:t xml:space=\"preserve\">{text}</w:t>"
    if br:
        inner += "<w:br/>"
    return f"<w:r>{inner}</w:r>"


def gen_docx():
    paras = [
        wpara(wrun("Contract Summary"), style="Heading1"),
        wpara(wrun("This agreement governs the use of the service.")),
        # Tracked changes: accepted view keeps w:ins, drops w:del.
        wpara(
            wrun("Base terms")
            + '<w:ins w:id="1" w:author="Reviewer">'
            + wrun(" inserted clause")
            + "</w:ins>"
            + '<w:del w:id="2" w:author="Reviewer">'
            + "<w:r><w:delText> removed clause</w:delText></w:r>"
            + "</w:del>"
        ),
        wpara(wrun("Payment terms follow the table below.")),
        # 2x2 table (minimal but schema-plausible).
        "<w:tbl><w:tblPr/><w:tblGrid/>"
        "<w:tr><w:tc>" + wpara(wrun("Price")) + "</w:tc><w:tc>"
        + wpara(wrun("Amount")) + "</w:tc></w:tr>"
        "<w:tr><w:tc>" + wpara(wrun("Base fee")) + "</w:tc><w:tc>"
        + wpara(wrun("100 USD")) + "</w:tc></w:tr>"
        "</w:tbl>",
        wpara(wrun("Contact: ", tab=True) + wrun("admin@example.com", br=True) + wrun("Phone line.")),
    ]
    document = (DOC_BODY_OPEN + "".join(paras) + "</w:body></w:document>").encode()
    header = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:hdr xmlns:w="{W_NS}"><w:p><w:r><w:t>CONFIDENTIAL</w:t></w:r></w:p></w:hdr>'
    ).encode()

    with zipfile.ZipFile(os.path.join(FIXTURES, "sample.docx"), "w") as z:
        add_zip_entry(
            z,
            "[Content_Types].xml",
            content_types(
                ("/word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"),
                ("/word/header1.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"),
            ),
        )
        add_zip_entry(
            z,
            "_rels/.rels",
            rels([("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument", "word/document.xml")]),
        )
        add_zip_entry(z, "word/document.xml", document)
        add_zip_entry(z, "word/header1.xml", header)

    # Empty document: one empty paragraph, zero blocks.
    empty = (DOC_BODY_OPEN + wpara("") + "</w:body></w:document>").encode()
    with zipfile.ZipFile(os.path.join(FIXTURES, "empty.docx"), "w") as z:
        add_zip_entry(
            z,
            "[Content_Types].xml",
            content_types(
                ("/word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"),
            ),
        )
        add_zip_entry(
            z,
            "_rels/.rels",
            rels([("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument", "word/document.xml")]),
        )
        add_zip_entry(z, "word/document.xml", empty)

    nfc = "Điều khoản thanh toán trong vòng 30 ngày."
    nfd = unicodedata.normalize("NFD", nfc)
    document = (DOC_BODY_OPEN + wpara(wrun(nfd)) + "</w:body></w:document>").encode()
    with zipfile.ZipFile(os.path.join(FIXTURES, "vietnamese.docx"), "w") as z:
        add_zip_entry(
            z,
            "[Content_Types].xml",
            content_types(
                ("/word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"),
            ),
        )
        add_zip_entry(
            z,
            "_rels/.rels",
            rels([("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument", "word/document.xml")]),
        )
        add_zip_entry(z, "word/document.xml", document)


# --------------------------------------------------------------------------
# XLSX fixture (2 visible sheets incl. a formula + 1 hidden sheet)
# --------------------------------------------------------------------------

CELL = '<c r="{ref}"{type}><v>{val}</v></c>'


def xcell(col, row, value, t="n"):
    ref = f"{chr(ord('A') + col)}{row}"
    if t == "s":
        return f'<c r="{ref}" t="inlineStr"><is><t>{value}</t></is></c>'
    if t == "f":
        return f'<c r="{ref}"><f>{value}</f><v>8</v></c>'
    return CELL.format(ref=ref, type="", val=value)


def gen_xlsx():
    sheet1 = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        "<sheetData>"
        '<row r="1">' + xcell(0, 1, "Item", "s") + xcell(1, 1, "Qty", "s") + "</row>"
        '<row r="2">' + xcell(0, 2, "Widgets", "s") + xcell(1, 2, 3) + "</row>"
        '<row r="3">' + xcell(0, 3, "Gadgets", "s") + xcell(1, 3, 5) + "</row>"
        '<row r="4">' + xcell(0, 4, "Total", "s") + xcell(1, 4, "SUM(B2:B3)", "f") + "</row>"
        "</sheetData></worksheet>"
    ).encode()
    sheet2 = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        "<sheetData>"
        '<row r="1">' + xcell(0, 1, "hello world", "s") + "</row>"
        "</sheetData></worksheet>"
    ).encode()

    ns = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    workbook = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        f'xmlns:r="{ns}"><sheets>'
        '<sheet name="Data" sheetId="1" r:id="rId1"/>'
        '<sheet name="Notes" sheetId="2" r:id="rId2"/>'
        '<sheet name="Secret" sheetId="3" state="hidden" r:id="rId3"/>'
        "</sheets></workbook>"
    ).encode()
    workbook_rels = rels([
        ("rId1", f"{ns}/worksheet", "worksheets/sheet1.xml"),
        ("rId2", f"{ns}/worksheet", "worksheets/sheet2.xml"),
        ("rId3", f"{ns}/worksheet", "worksheets/sheet3.xml"),
    ])

    with zipfile.ZipFile(os.path.join(FIXTURES, "sample.xlsx"), "w") as z:
        add_zip_entry(
            z,
            "[Content_Types].xml",
            content_types(
                ("/xl/workbook.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"),
                ("/xl/worksheets/sheet1.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
                ("/xl/worksheets/sheet2.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
                ("/xl/worksheets/sheet3.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
            ),
        )
        add_zip_entry(
            z,
            "_rels/.rels",
            rels([("rId1", f"{ns}/officeDocument", "xl/workbook.xml")]),
        )
        add_zip_entry(z, "xl/workbook.xml", workbook)
        add_zip_entry(z, "xl/_rels/workbook.xml.rels", workbook_rels)
        add_zip_entry(z, "xl/worksheets/sheet1.xml", sheet1)
        add_zip_entry(z, "xl/worksheets/sheet2.xml", sheet2)
        # sheet3 (hidden) intentionally missing data — calamine skips it.


# --------------------------------------------------------------------------
# PPTX fixture (3 slides, title placeholder on slides 1 & 3, notes on 3)
# --------------------------------------------------------------------------

P_NS = "http://schemas.openxmlformats.org/presentationml/2006/main"
A_NS = "http://schemas.openxmlformats.org/drawingml/2006/main"
R_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"


def slide_xml(title, body_lines):
    def shape(text, ph_type=None):
        ph = f'<p:ph type="{ph_type}"/>' if ph_type else ""
        return (
            f'<p:sp><p:nvSpPr><p:cNvPr id="1" name="S"/>'
            f"<p:nvPr>{ph}</p:nvPr></p:nvSpPr>"
            '<p:txBody><a:bodyPr/><a:lstStyle/>'
            f'<a:p><a:r><a:t>{text}</a:t></a:r></a:p>'
            "</p:txBody></p:sp>"
        )

    body_shapes = "".join(
        '<p:sp><p:nvSpPr><p:cNvPr id="2" name="B"/><p:nvPr/></p:nvSpPr>'
        '<p:txBody><a:bodyPr/><a:lstStyle/>'
        f'<a:p><a:r><a:t>{line}</a:t></a:r></a:p>'
        "</p:txBody></p:sp>"
        for line in body_lines
    )
    title_shape = shape(title, "title") if title else ""
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<p:sld xmlns:p="{P_NS}" xmlns:a="{A_NS}" xmlns:r="{R_NS}">'
        f'<p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>'
        + title_shape + body_shapes + "</p:spTree></p:cSld></p:sld>"
    ).encode()


def gen_pptx():
    with zipfile.ZipFile(os.path.join(FIXTURES, "sample.pptx"), "w") as z:
        slide_parts = [
            ("ppt/slides/slide1.xml", slide_xml("Quarterly Review", ["Revenue is up.", "Costs are flat."])),
            ("ppt/slides/slide2.xml", slide_xml(None, ["Plain slide text."])),
            ("ppt/slides/slide3.xml", slide_xml("Next Steps", ["Ship phase 2."])),
        ]
        overrides = [
            ("/ppt/presentation.xml", "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"),
            ("ppt/notesSlides/notesSlide1.xml", "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"),
        ] + [(p, "application/vnd.openxmlformats-officedocument.presentationml.slide+xml") for p, _ in slide_parts]
        add_zip_entry(z, "[Content_Types].xml", content_types(*overrides))
        add_zip_entry(
            z,
            "_rels/.rels",
            rels([("rId1", f"{R_NS}/officeDocument", "ppt/presentation.xml")]),
        )
        add_zip_entry(
            z,
            "ppt/presentation.xml",
            (
                '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
                f'<p:presentation xmlns:p="{P_NS}" xmlns:r="{R_NS}">'
                '<p:sldIdLst>'
                '<p:sldId id="1" r:id="rId1"/>'
                '<p:sldId id="2" r:id="rId2"/>'
                '<p:sldId id="3" r:id="rId3"/>'
                '</p:sldIdLst></p:presentation>'
            ).encode(),
        )
        add_zip_entry(
            z,
            "ppt/_rels/presentation.xml.rels",
            rels([(f"rId{i}", f"{R_NS}/slide", f"slides/slide{i}.xml") for i in (1, 2, 3)]),
        )
        for path, data in slide_parts:
            add_zip_entry(z, path, data)
        add_zip_entry(
            z,
            "ppt/notesSlides/notesSlide1.xml",
            (
                '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
                f'<p:notes xmlns:p="{P_NS}" xmlns:a="{A_NS}">'
                '<p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Remember the demo.</a:t>'
                "</a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"
            ).encode(),
        )


# --------------------------------------------------------------------------
# Zip-bomb fixture (hand-crafted zip: huge declared size, tiny payload)
# --------------------------------------------------------------------------

def gen_bomb_docx():
    def deflate(data):
        compressor = zlib.compressobj(level=9, method=zlib.DEFLATED, wbits=-15)
        return compressor.compress(data) + compressor.flush()

    huge_declared = 1 << 30  # 1 GiB "uncompressed"

    # (name, real uncompressed content, declared uncompressed size)
    entries = [
        ("[Content_Types].xml", content_types(), len(content_types())),
        ("word/document.xml", b"<fake/>", huge_declared),  # bomb entry
    ]

    def local_header(name, comp_size, uncomp_size, crc):
        return (
            struct.pack(
                "<4s5H3L2H",
                b"PK\x03\x04", 20, 0, 8, 0, 0, crc, comp_size, uncomp_size,
                len(name), 0,
            )
            + name.encode()
        )

    def central_header(name, comp_size, uncomp_size, crc, offset):
        return (
            struct.pack(
                "<4s6H3L5H2L",
                b"PK\x01\x02", 20, 20, 0, 8, 0, 0, crc, comp_size, uncomp_size,
                len(name), 0, 0, 0, 0, 0, offset,
            )
            + name.encode()
        )

    out = bytearray()
    central = bytearray()
    offset = 0
    for name, real_data, uncomp_size in entries:
        comp = deflate(real_data)
        crc = zlib.crc32(real_data)
        out += local_header(name, len(comp), uncomp_size, crc) + comp
        central += central_header(name, len(comp), uncomp_size, crc, offset)
        offset += len(local_header(name, len(comp), uncomp_size, crc)) + len(comp)
    cd_offset = len(out)
    out += central
    out += struct.pack(
        "<4s4H2LH", b"PK\x05\x06", 0, 0, len(entries), len(entries),
        len(central), cd_offset, 0,
    )
    write(os.path.join(FIXTURES, "bomb.docx"), bytes(out))


if __name__ == "__main__":
    gen_txt()
    gen_pdf()
    gen_docx()
    gen_xlsx()
    gen_pptx()
    gen_bomb_docx()
    print("all fixtures generated")
