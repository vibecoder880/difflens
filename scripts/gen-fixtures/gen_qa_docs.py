#!/usr/bin/env python3
"""Generate the QA document pairs consumed by core/tests/qa_pipeline.rs and
core/tests/qa_loader_chain.rs.

Deterministic (fixed zip timestamps, fixed PDF layout, no wall-clock or
randomness), stdlib-only, self-sufficient: run

    python3 -I scripts/gen-fixtures/gen_qa_docs.py [output-dir]

(default output: <repo>/target/qa-docs). The test harness re-runs this
automatically when the `.generated` marker is missing; the marker is written
last so a partially-generated directory is always regenerated.

Document set:
- contract-v1.docx / contract-v2.docx — Vietnamese service contract: 17
  paragraph blocks per side, 11 equal, 2 moved (delivery + warranty clause),
  3 modified (payment terms 30→15 days + 0,05%→0,075%, penalty money
  5.000.000₫→12.000.000₫, renamed confidentiality heading), 1 removed
  (progress-report clause), 1 added (confidentiality clause with a 3-year
  duration entity).
- contract-v1.pdf / contract-v2.pdf — the same contract drawn with an
  embedded TrueType font (DejaVuSans, Identity-H + ToUnicode) so Vietnamese
  diacritics survive pdfium extraction. The font is read from the system
  (candidates below); when none exists the PDFs are skipped and a
  `.pdf-font-missing` marker is written instead (PDF tests then skip).
- price-v1.xlsx / price-v2.xlsx — one visible sheet "Bảng giá", 32 cell
  blocks; blocks[4] is Cell(row=1, col=1) = "1.500.000₫" → "2.000.000₫"
  (MONEY_CHANGED +1/3), plus REFERENCE_CHANGED 2026/HD-001→002 and a CPI
  note-cell edit (3 modified, 29 equal).
- deck-v1.pptx / deck-v2.pptx — 3 slides: slide 1 (title anchor "Hợp đồng
  dịch vụ") changes 6 tháng→4 tháng, slide 2 gains a "Giai đoạn 3" milestone
  line, slide 3 is identical.
- contract-md-v1.md / contract-md-v2.md — markdown pair: the payment
  paragraph (two non-blank lines joined by the extractor) changes 30→45
  ngày, a late-interest clause gains "0,1%/ngày", and a hostile clause
  (script tag, img onerror, manual ``` fence) is added verbatim.
"""

import os
import re
import struct
import sys
import unicodedata
import zipfile

ZIP_DATE = (2026, 1, 1, 0, 0, 0)

# Fonts probed for the PDF builder (first hit wins). The canonical Debian/
# Ubuntu path (fonts-dejavu-core, present on CI ubuntu-24.04 runners and dev
# VMs) comes first; the glob fallback covers other distributions.
FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/local/share/fonts/DejaVuSans.ttf",
]


def find_font():
    for path in FONT_CANDIDATES:
        if os.path.isfile(path):
            return path
    for base in ("/usr/share/fonts", "/usr/local/share/fonts", os.path.expanduser("~/.fonts")):
        if not os.path.isdir(base):
            continue
        for root, _, files in os.walk(base):
            for name in sorted(files):
                if name == "DejaVuSans.ttf":
                    return os.path.join(root, name)
    return None


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    print("wrote", path, len(data), "bytes")


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


def xml_escape(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def nfc(text):
    return unicodedata.normalize("NFC", text)


# ===========================================================================
# Shared contract content (see module docstring for the expected diff shape)
# ===========================================================================

TITLE = "HỢP ĐỒNG CUNG CẤP PHẦN MỀM QUẢN LÝ BÁN HÀNG"
PREAMBLE = (
    "Căn cứ Bộ luật Dân sự năm 2015 và nhu cầu hợp tác của hai bên, các bên "
    "ký kết hợp đồng này với những điều khoản dưới đây."
)
D1 = "Điều 1. Thông tin chung"
BEN_A = (
    "Bên A: Công ty TNHH Công nghệ Ánh Sáng, địa chỉ 12 Nguyễn Huệ, Quận 1, "
    "TP. Hồ Chí Minh."
)
BEN_B = (
    "Bên B: Công ty CP Thương mại An Phát, địa chỉ 45 Lê Lợi, Quận 1, "
    "TP. Hồ Chí Minh."
)
D2 = "Điều 2. Phạm vi công việc"
DELIVERY = (
    "Bên A bàn giao phần mềm quản lý bán hàng kèm tài liệu hướng dẫn sử "
    "dụng cho Bên B."
)
D3 = "Điều 3. Giá trị hợp đồng"
VALUE = (
    "Tổng giá trị hợp đồng là 250.000.000₫ (hai trăm năm mươi triệu Việt "
    "Nam đồng), đã bao gồm thuế giá trị gia tăng."
)
D4 = "Điều 4. Thanh toán"
PAYMENT_V1 = (
    "Thanh toán trong vòng 30 ngày kể từ ngày xuất hóa đơn; lãi chậm trả "
    "0,05%/ngày."
)
PAYMENT_V2 = (
    "Thanh toán trong vòng 15 ngày kể từ ngày xuất hóa đơn; lãi chậm trả "
    "0,075%/ngày, thêm phí quản lý nợ 0,075%/ngày."
)
D5 = "Điều 5. Phạt vi phạm"
PENALTY_V1 = (
    "Mức phạt vi phạm nghĩa vụ thanh toán: 5.000.000₫ cho mỗi lần chậm "
    "thanh toán quá hạn."
)
PENALTY_V2 = (
    "Mức phạt vi phạm nghĩa vụ thanh toán: 12.000.000₫ cho mỗi lần chậm "
    "thanh toán quá hạn."
)
D6 = "Điều 6. Trách nhiệm bảo hành sản phẩm"
D7_V1 = "Điều 7. Trách nhiệm bảo mật"
D7_V2 = "Điều 7. Trách nhiệm bảo mật và bồi thường thiệt hại"
D8 = "Điều 8. Điều khoản chung khác"
PROGRESS = (
    "Vào thứ Sáu cuối mỗi tháng, Bên A gửi báo cáo tiến độ định kỳ cho Bên B "
    "qua thư điện tử."
)
CONFID = (
    "Bảo mật thông tin: Bên A cam kết không tiết lộ dữ liệu của Bên B trong "
    "thời hạn 3 năm kể từ khi hợp đồng hết hạn."
)

CONTRACT_V1 = [
    (TITLE, "h"),
    (PREAMBLE, "p"),
    (D1, "h"),
    (BEN_A, "p"),
    (BEN_B, "p"),
    (D2, "h"),
    (DELIVERY, "p"),
    (D3, "h"),
    (VALUE, "p"),
    (D4, "h"),
    (PAYMENT_V1, "p"),
    (D5, "h"),
    (PENALTY_V1, "p"),
    (D6, "h"),
    (D7_V1, "h"),
    (D8, "h"),
    (PROGRESS, "p"),
]

CONTRACT_V2 = [
    (TITLE, "h"),
    (PREAMBLE, "p"),
    (D1, "h"),
    (BEN_A, "p"),
    (BEN_B, "p"),
    (D2, "h"),
    (D3, "h"),
    (VALUE, "p"),
    (D4, "h"),
    (PAYMENT_V2, "p"),
    (D5, "h"),
    (PENALTY_V2, "p"),
    (D7_V2, "h"),
    (CONFID, "p"),
    (D8, "h"),
    (D6, "h"),
    (DELIVERY, "p"),
]

assert len(CONTRACT_V1) == len(CONTRACT_V2) == 17


# ===========================================================================
# DOCX (one block per non-empty paragraph, Heading1 style for headings)
# ===========================================================================

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
DOC_BODY_OPEN = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    f'<w:document xmlns:w="{W_NS}"><w:body>'
)


def wpara(inner, style=None):
    style_xml = f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else ""
    return f"<w:p>{style_xml}{inner}</w:p>"


def wrun(text):
    text = xml_escape(text)
    return f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'


def gen_contract_docx(path, paras):
    body = "".join(
        wpara(wrun(text), style="Heading1" if kind == "h" else None)
        for text, kind in paras
    )
    document = (DOC_BODY_OPEN + body + "</w:body></w:document>").encode()
    with zipfile.ZipFile(path, "w") as z:
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


# ===========================================================================
# XLSX (single visible sheet "Bảng giá", inlineStr cells)
# ===========================================================================

def xcell(col, row, value):
    # col is 0-based (A=0), row is 1-based; matches gen-fixtures/generate.py.
    ref = f"{chr(ord('A') + col)}{row}"
    return f'<c r="{ref}" t="inlineStr"><is><t>{xml_escape(value)}</t></is></c>'


PRICE_SHEET = "Bảng giá"
# Grid rows in reading order; None = empty cell (skipped by the extractor so
# that blocks[4] lands on Cell(row=1, col=1)).
PRICE_GRID_V1 = [
    ["Mã", "Đơn giá", "Hạng mục", "Ghi chú"],
    [None, "1.500.000₫", "Phí triển khai ban đầu", "Một lần"],
    ["2", "3.600.000₫", "Bản quyền sử dụng năm thứ hai", "Hằng năm"],
    ["3", "2.400.000₫", "Hỗ trợ kỹ thuật chuyên sâu", "Mỗi lần yêu cầu"],
    ["4", "1.800.000₫", "Đào tạo sử dụng phần mềm", "Hai buổi"],
    ["5", "900.000₫", "Bảo trì hệ thống định kỳ", "Hằng quý"],
    ["6", "Số hợp đồng: 2026/HD-001", "Áp dụng cho hợp đồng gốc", None],
    ["7", "Chính sách điều chỉnh", "Giá điều chỉnh theo biến động thị trường hằng năm", None],
    ["8", "Ngày hiệu lực", "01/01/2026", None],
]
PRICE_GRID_V2 = [
    ["Mã", "Đơn giá", "Hạng mục", "Ghi chú"],
    [None, "2.000.000₫", "Phí triển khai ban đầu", "Một lần"],
    ["2", "3.600.000₫", "Bản quyền sử dụng năm thứ hai", "Hằng năm"],
    ["3", "2.400.000₫", "Hỗ trợ kỹ thuật chuyên sâu", "Mỗi lần yêu cầu"],
    ["4", "1.800.000₫", "Đào tạo sử dụng phần mềm", "Hai buổi"],
    ["5", "900.000₫", "Bảo trì hệ thống định kỳ", "Hằng quý"],
    ["6", "Số hợp đồng: 2026/HD-002", "Áp dụng cho hợp đồng gốc", None],
    ["7", "Chính sách điều chỉnh", "Giá điều chỉnh theo chỉ số CPI hằng năm", None],
    ["8", "Ngày hiệu lực", "01/01/2026", None],
]

REL_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"


def gen_price_xlsx(path, grid):
    rows = []
    for r, cells in enumerate(grid, start=1):
        cells_xml = "".join(
            xcell(c, r, value) for c, value in enumerate(cells) if value is not None
        )
        rows.append(f'<row r="{r}">{cells_xml}</row>')
    sheet1 = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        "<sheetData>" + "".join(rows) + "</sheetData></worksheet>"
    ).encode()
    workbook = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        f'xmlns:r="{REL_NS}"><sheets>'
        f'<sheet name="{xml_escape(PRICE_SHEET)}" sheetId="1" r:id="rId1"/>'
        "</sheets></workbook>"
    ).encode()
    with zipfile.ZipFile(path, "w") as z:
        add_zip_entry(
            z,
            "[Content_Types].xml",
            content_types(
                ("/xl/workbook.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"),
                ("/xl/worksheets/sheet1.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
            ),
        )
        add_zip_entry(
            z,
            "_rels/.rels",
            rels([( "rId1", f"{REL_NS}/officeDocument", "xl/workbook.xml")]),
        )
        add_zip_entry(z, "xl/workbook.xml", workbook)
        add_zip_entry(
            z,
            "xl/_rels/workbook.xml.rels",
            rels([("rId1", f"{REL_NS}/worksheet", "worksheets/sheet1.xml")]),
        )
        add_zip_entry(z, "xl/worksheets/sheet1.xml", sheet1)


# ===========================================================================
# PPTX (3 slides, title placeholder on each)
# ===========================================================================

P_NS = "http://schemas.openxmlformats.org/presentationml/2006/main"
A_NS = "http://schemas.openxmlformats.org/drawingml/2006/main"


def slide_xml(title, body_lines):
    def para(text):
        return f"<a:p><a:r><a:t>{xml_escape(text)}</a:t></a:r></a:p>"

    title_shape = (
        '<p:sp><p:nvSpPr><p:cNvPr id="1" name="T"/>'
        '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>'
        '<p:txBody><a:bodyPr/><a:lstStyle/>'
        + para(title)
        + "</p:txBody></p:sp>"
    )
    body_shape = (
        '<p:sp><p:nvSpPr><p:cNvPr id="2" name="B"/><p:nvPr/></p:nvSpPr>'
        '<p:txBody><a:bodyPr/><a:lstStyle/>'
        + "".join(para(line) for line in body_lines)
        + "</p:txBody></p:sp>"
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<p:sld xmlns:p="{P_NS}" xmlns:a="{A_NS}" xmlns:r="{REL_NS}">'
        '<p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>'
        + title_shape + body_shape + "</p:spTree></p:cSld></p:sld>"
    ).encode()


DECK_SLIDES_V1 = [
    ("Hợp đồng dịch vụ", [
        "Thời gian thực hiện: 6 tháng",
        "Chi phí cố định không thay đổi trong quá trình thực hiện",
    ]),
    ("Phạm vi công việc", [
        "Giai đoạn 1: Khảo sát và phân tích yêu cầu",
        "Giai đoạn 2: Thiết kế và triển khai",
        "Kiểm thử toàn diện trước khi bàn giao",
    ]),
    ("Liên hệ và hỗ trợ", [
        "Kênh hỗ trợ chính thức qua thư điện tử của Bên A",
    ]),
]
DECK_SLIDES_V2 = [
    ("Hợp đồng dịch vụ", [
        "Thời gian thực hiện: 4 tháng",
        "Chi phí cố định không thay đổi trong quá trình thực hiện",
    ]),
    ("Phạm vi công việc", [
        "Giai đoạn 1: Khảo sát và phân tích yêu cầu",
        "Giai đoạn 2: Thiết kế và triển khai",
        "Giai đoạn 3: Đào tạo và chuyển giao",
        "Kiểm thử toàn diện trước khi bàn giao",
    ]),
    ("Liên hệ và hỗ trợ", [
        "Kênh hỗ trợ chính thức qua thư điện tử của Bên A",
    ]),
]


def gen_deck_pptx(path, slides):
    with zipfile.ZipFile(path, "w") as z:
        slide_parts = [
            (f"ppt/slides/slide{i}.xml", slide_xml(title, body))
            for i, (title, body) in enumerate(slides, start=1)
        ]
        overrides = [
            ("/ppt/presentation.xml", "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"),
        ] + [
            (p, "application/vnd.openxmlformats-officedocument.presentationml.slide+xml")
            for p, _ in slide_parts
        ]
        add_zip_entry(z, "[Content_Types].xml", content_types(*overrides))
        add_zip_entry(
            z,
            "_rels/.rels",
            rels([("rId1", f"{REL_NS}/officeDocument", "ppt/presentation.xml")]),
        )
        sld_ids = "".join(
            f'<p:sldId id="{i}" r:id="rId{i}"/>' for i in range(1, len(slides) + 1)
        )
        add_zip_entry(
            z,
            "ppt/presentation.xml",
            (
                '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
                f'<p:presentation xmlns:p="{P_NS}" xmlns:r="{REL_NS}">'
                f'<p:sldIdLst>{sld_ids}</p:sldIdLst></p:presentation>'
            ).encode(),
        )
        add_zip_entry(
            z,
            "ppt/_rels/presentation.xml.rels",
            rels([(f"rId{i}", f"{REL_NS}/slide", f"slides/slide{i}.xml") for i in range(1, len(slides) + 1)]),
        )
        for part_path, data in slide_parts:
            add_zip_entry(z, part_path, data)


# ===========================================================================
# Markdown pair
# ===========================================================================

MD_PAYMENT_OLD = (
    "Bên B thanh toán trong vòng 30 ngày kể từ ngày xuất hóa đơn.\n"
    "Mọi khoản thanh toán chuyển khoản vào tài khoản Vietcombank chi nhánh "
    "Quận 1."
)
MD_PAYMENT_NEW = MD_PAYMENT_OLD.replace("30 ngày", "45 ngày")


def gen_contract_md(path, payment, with_hostile, with_cpi):
    late = (
        "Trả chậm bị tính lãi 0,1%/ngày trên số tiền còn thiếu."
        if with_cpi
        else "Trả chậm bị tính lãi phạt theo quy định chung của hợp đồng."
    )
    parts = [
        "# Hợp đồng dịch vụ phần mềm",
        "",
        "## Điều 1. Thanh toán",
        "",
        payment,
        "",
        "## Điều 2. Lãi chậm thanh toán",
        "",
        late,
        "",
        "## Điều 3. Giải quyết tranh chấp",
        "",
        "Mọi tranh chấp được giải quyết thông qua thương lượng trước khi khởi kiện.",
        "",
    ]
    if with_hostile:
        parts.append("<script>alert('dl-xss')</script> <img src=x onerror=alert(2)> và khối ```fence thủ công``` cố ý")
        parts.append("")
    write(path, "\n".join(parts).encode())


# ===========================================================================
# PDF: Identity-H embedded TrueType (DejaVuSans) so Vietnamese diacritics
# survive pdfium. Full font file embedded as /FontFile2; CIDs are glyph ids.
# ===========================================================================

def parse_ttf(data):
    """Minimal sfnt parser: cmap (format 4 or 12), hmtx advances, metrics."""
    num_tables = struct.unpack(">H", data[4:6])[0]
    tables = {}
    for i in range(num_tables):
        off = 12 + 16 * i
        tag = data[off:off + 4]
        t_off, t_len = struct.unpack(">II", data[off + 8:off + 16])
        tables[tag] = (t_off, t_len)

    head_off = tables[b"head"][0]
    upem = struct.unpack(">H", data[head_off + 18:head_off + 20])[0]
    bbox = struct.unpack(">hhhh", data[head_off + 36:head_off + 44])

    hhea_off = tables[b"hhea"][0]
    ascent, descent = struct.unpack(">hh", data[hhea_off + 4:hhea_off + 8])
    num_h_metrics = struct.unpack(">H", data[hhea_off + 34:hhea_off + 36])[0]

    hmtx_off = tables[b"hmtx"][0]
    advances = [
        struct.unpack(">H", data[hmtx_off + 4 * i:hmtx_off + 4 * i + 2])[0]
        for i in range(num_h_metrics)
    ]

    def cmap_format4(off):
        segcount_x2 = struct.unpack(">H", data[off + 6:off + 8])[0]
        segcount = segcount_x2 // 2
        end_base = off + 14
        start_base = end_base + segcount_x2 + 2
        delta_base = start_base + segcount_x2
        range_base = delta_base + segcount_x2
        out = {}
        for i in range(segcount):
            end = struct.unpack(">H", data[end_base + 2 * i:end_base + 2 * i + 2])[0]
            start = struct.unpack(">H", data[start_base + 2 * i:start_base + 2 * i + 2])[0]
            delta = struct.unpack(">h", data[delta_base + 2 * i:delta_base + 2 * i + 2])[0]
            ro = struct.unpack(">H", data[range_base + 2 * i:range_base + 2 * i + 2])[0]
            if start == 0xFFFF:
                continue
            for c in range(start, end + 1):
                if ro == 0:
                    out[c] = (c + delta) & 0xFFFF
                else:
                    addr = range_base + 2 * i + ro + 2 * (c - start)
                    gid = struct.unpack(">H", data[addr:addr + 2])[0]
                    if gid:
                        out[c] = gid
        return out

    def cmap_format12(off):
        n = struct.unpack(">I", data[off + 12:off + 16])[0]
        out = {}
        for i in range(n):
            s, e, g = struct.unpack(">III", data[off + 16 + 12 * i:off + 28 + 12 * i])
            for c in range(s, min(e, 0xFFFF) + 1):
                out[c] = g + (c - s)
        return out

    cmap_off = tables[b"cmap"][0]
    sub_count = struct.unpack(">H", data[cmap_off + 2:cmap_off + 4])[0]
    subtables = []
    for i in range(sub_count):
        pid, eid, off = struct.unpack(">HHI", data[cmap_off + 4 + 8 * i:cmap_off + 12 + 8 * i])
        subtables.append((pid, eid, cmap_off + off))

    code_to_gid = None
    # Full-unicode subtable first, then BMP-only fallbacks.
    for pid, eid, off in subtables:
        if (pid, eid) in ((3, 10), (0, 4), (0, 6)):
            fmt = struct.unpack(">H", data[off:off + 2])[0]
            if fmt == 12:
                code_to_gid = cmap_format12(off)
                break
    if code_to_gid is None:
        for pid, eid, off in subtables:
            if (pid, eid) in ((3, 1), (0, 3), (3, 0), (0, 2), (0, 1)):
                fmt = struct.unpack(">H", data[off:off + 2])[0]
                if fmt == 4:
                    code_to_gid = cmap_format4(off)
                    break
    if code_to_gid is None:
        raise RuntimeError("no usable cmap subtable in font")

    return {
        "upem": upem,
        "bbox": bbox,
        "ascent": ascent,
        "descent": descent,
        "advances": advances,
        "cmap": code_to_gid,
    }


class PdfFont:
    """Unicode text → Identity-H glyph ids + widths for one TTF."""

    def __init__(self, font_path):
        with open(font_path, "rb") as f:
            self.raw = f.read()
        info = parse_ttf(self.raw)
        self.info = info
        self.upem = info["upem"]
        self.cmap = info["cmap"]
        self.advances = info["advances"]
        self.used = {}  # codepoint → gid

    def glyph(self, ch):
        code = ord(ch)
        if code not in self.used:
            gid = self.cmap.get(code)
            if not gid:
                raise RuntimeError(f"font has no glyph for U+{code:04X} ({ch!r})")
            self.used[code] = gid
        return self.used[code]

    def width(self, text, size):
        total = 0
        for ch in text:
            gid = self.glyph(ch)
            adv = self.advances[gid] if gid < len(self.advances) else self.advances[-1]
            total += adv
        return total * size / self.upem

    def hex_string(self, text):
        return "".join(f"{self.glyph(ch):04X}" for ch in text)

    def widths_object(self):
        parts = []
        for code in sorted(self.used):
            gid = self.used[code]
            adv = self.advances[gid] if gid < len(self.advances) else self.advances[-1]
            w = round(adv * 1000 / self.upem)
            parts.append(f"{gid} [{w}]")
        return "[" + " ".join(parts) + "]"

    def to_unicode_stream(self):
        # Reverse cmap restricted to used glyphs; bfchar blocks of ≤100.
        entries = []
        for code in sorted(self.used):
            gid = self.used[code]
            u16 = struct.pack(">H", code).hex().upper()
            entries.append(f"<{gid:04X}> <{u16}>")
        blocks = []
        for i in range(0, len(entries), 100):
            chunk = "\n".join(entries[i:i + 100])
            blocks.append(f"{len(entries[i:i + 100])} beginbfchar\n{chunk}\nendbfchar")
        body = (
            "/CIDInit /ProcSet findresource begin\n"
            "12 dict begin\n"
            "begincmap\n"
            "/CMapName /DejaVuSans-H def\n"
            "/CMapType 2 def\n"
            "1 begincodespacerange\n"
            "<0000> <FFFF>\n"
            "endcodespacerange\n"
            + "\n".join(blocks) + "\n"
            "endcmap\n"
            "CMapName currentdict /CMap defineresource pop\n"
            "end\n"
            "end\n"
        )
        return body.encode("latin-1")


PAGE_W, PAGE_H = 1200, 842
MARGIN_X = 56
TOP_BASELINE = 780
BLOCK_ADVANCE = 50
BOTTOM_LIMIT = 60
FONT_SIZE = 11


def gen_contract_pdf(path, paras, font):
    # Layout: one visual line per paragraph (page is wide enough), generous
    # gaps so the extractor's paragraph merge keeps blocks separated.
    lines = []  # (page_index, y, text)
    page = 0
    y = TOP_BASELINE
    for text, _kind in paras:
        if y < BOTTOM_LIMIT:
            page += 1
            y = TOP_BASELINE
        assert font.width(text, FONT_SIZE) <= PAGE_W - 2 * MARGIN_X, text
        lines.append((page, y, text))
        y -= BLOCK_ADVANCE
    page_count = page + 1

    def content_stream(plines):
        ops = ["BT"]
        for _, y, text in plines:
            ops.append(f"1 0 0 1 {MARGIN_X} {y} Tm")
            ops.append(f"/F1 {FONT_SIZE} Tf")
            ops.append(f"<{font.hex_string(text)}> Tj")
        ops.append("ET")
        return "\n".join(ops).encode("latin-1")

    page_ids = []
    page_objs = []
    for p in range(page_count):
        plines = [(pg, y, t) for pg, y, t in lines if pg == p]
        stream = content_stream(plines)
        page_id = 8 + 2 * p
        content_id = page_id + 1
        page_ids.append(page_id)
        page_objs.append(
            (
                page_id,
                f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] "
                f"/Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>".encode(),
            )
        )
        page_objs.append(
            (content_id, b"<< /Length %d >>\nstream\n%b\nendstream" % (len(stream), stream))
        )

    gid_stream = font.to_unicode_stream()
    x0, y0, x1, y1 = font.info["bbox"]
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",  # 1
        f"<< /Type /Pages /Kids [{' '.join(f'{pid} 0 R' for pid in page_ids)}] "
        f"/Count {page_count} >>".encode(),  # 2
        b"<< /Type /Font /Subtype /Type0 /BaseFont /DejaVuSans /Encoding /Identity-H "
        b"/DescendantFonts [4 0 R] /ToUnicode 5 0 R >>",  # 3
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /DejaVuSans "
        b"/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> "
        b"/FontDescriptor 6 0 R /DW 1000 /W " + font.widths_object().encode() +
        b" /CIDToGIDMap /Identity >>",  # 4
        b"<< /Length %d >>\nstream\n%b\nendstream" % (len(gid_stream), gid_stream),  # 5
        f"<< /Type /FontDescriptor /FontName /DejaVuSans /Flags 32 "
        f"/FontBBox [{x0} {y0} {x1} {y1}] /ItalicAngle 0 /Ascent {font.info['ascent']} "
        f"/Descent {font.info['descent']} /CapHeight 1493 /StemV 80 "
        f"/FontFile2 7 0 R >>".encode(),  # 6
        b"<< /Length %d /Length1 %d >>\nstream\n%b\nendstream"
        % (len(font.raw), len(font.raw), font.raw),  # 7
    ]
    objects.extend(body for _, body in page_objs)

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
    write(path, bytes(out))


# ===========================================================================
# Main
# ===========================================================================

def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
        "target", "qa-docs",
    )
    os.makedirs(out_dir, exist_ok=True)
    print(f"generating QA documents into {out_dir}")

    gen_contract_docx(os.path.join(out_dir, "contract-v1.docx"), CONTRACT_V1)
    gen_contract_docx(os.path.join(out_dir, "contract-v2.docx"), CONTRACT_V2)

    gen_price_xlsx(os.path.join(out_dir, "price-v1.xlsx"), PRICE_GRID_V1)
    gen_price_xlsx(os.path.join(out_dir, "price-v2.xlsx"), PRICE_GRID_V2)

    gen_deck_pptx(os.path.join(out_dir, "deck-v1.pptx"), DECK_SLIDES_V1)
    gen_deck_pptx(os.path.join(out_dir, "deck-v2.pptx"), DECK_SLIDES_V2)

    gen_contract_md(os.path.join(out_dir, "contract-md-v1.md"), MD_PAYMENT_OLD, False, False)
    gen_contract_md(os.path.join(out_dir, "contract-md-v2.md"), MD_PAYMENT_NEW, True, True)

    font_path = find_font()
    if font_path is None:
        # Rare (CI + dev VMs ship DejaVu): leave a marker so the PDF tests
        # skip gracefully instead of failing on a missing fixture.
        write(os.path.join(out_dir, ".pdf-font-missing"),
              b"no Vietnamese-capable TTF found; PDF fixtures skipped\n")
        print("WARNING: no DejaVuSans.ttf found — PDF fixtures skipped")
    else:
        print("using font:", font_path)
        font = PdfFont(font_path)
        gen_contract_pdf(os.path.join(out_dir, "contract-v1.pdf"), CONTRACT_V1, font)
        gen_contract_pdf(os.path.join(out_dir, "contract-v2.pdf"), CONTRACT_V2, font)
        marker = os.path.join(out_dir, ".pdf-font-missing")
        if os.path.exists(marker):
            os.remove(marker)

    write(os.path.join(out_dir, ".generated"), b"ok\n")
    print("QA documents generated")


if __name__ == "__main__":
    main()
