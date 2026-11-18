#!/usr/bin/env python3
"""Generate the hand-labeled gold alignment corpus + manifest.

Outputs into core/tests/gold/:
  - TXT and DOCX document pairs (deterministic, tiny, committed)
  - manifest.json with per-case expected block lists, gold aligned pairs
    and moved pairs, plus the recorded alignment configuration and the
    quality gates for the real (e5-small) and fake embedders.

All gold pairs are hand-constructed here: every (old_idx, new_idx) entry is
declared explicitly and cross-checked against the generated block lists
before the manifest is written.

Usage: python3 -I scripts/gen-fixtures/gen_gold.py
"""

import json
import os
import re
import unicodedata
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
GOLD = os.path.join(ROOT, "core", "tests", "gold")
ZIP_DATE = (2026, 1, 1, 0, 0, 0)

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"


# --------------------------------------------------------------------------
# helpers (mirroring core extraction exactly)
# --------------------------------------------------------------------------

def rust_normalize(text):
    """Mirror core/src/extract/normalize.rs: NFC + whitespace collapse + trim."""
    composed = unicodedata.normalize("NFC", text)
    collapsed = re.sub(r"\s+", " ", composed)
    return collapsed.strip()


def txt_blocks(markdown):
    """Mirror core/src/extract/txt.rs::split_blocks."""
    blocks = []
    paragraph = []

    def flush():
        if paragraph:
            text = rust_normalize("\n".join(paragraph))
            if text:
                blocks.append(text)
            paragraph.clear()

    for line in markdown.splitlines():
        stripped = line.lstrip(" ")
        hashes = len(stripped) - len(stripped.lstrip("#"))
        rest = stripped[hashes:]
        if 1 <= hashes <= 6 and (rest == "" or rest[0] in " \t"):
            flush()
            text = rust_normalize(stripped[hashes:])
            if text:
                blocks.append(text)
        elif not line.strip():
            flush()
        else:
            paragraph.append(line)
    flush()
    return blocks


def docx_write(path, paragraphs):
    """Write a minimal DOCX; one w:p per (style, text) entry, in order."""
    def wpara(style, text):
        style_xml = f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else ""
        return (
            f"<w:p>{style_xml}<w:r>"
            f'<w:t xml:space="preserve">{text}</w:t></w:r></w:p>'
        )

    body = "".join(wpara(style, text) for style, text in paragraphs)
    document = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:document xmlns:w="{W_NS}"><w:body>{body}</w:body></w:document>'
    ).encode()
    content_types = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType='
        '"application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/word/document.xml" ContentType='
        '"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
        "</Types>"
    ).encode()
    rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type='
        '"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" '
        'Target="word/document.xml"/></Relationships>'
    ).encode()

    with zipfile.ZipFile(path, "w") as z:
        for name, data in (
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", rels),
            ("word/document.xml", document),
        ):
            info = zipfile.ZipInfo(name, date_time=ZIP_DATE)
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, data)


def docx_blocks(paragraphs):
    return [rust_normalize(t) for (_s, t) in paragraphs if rust_normalize(t)]


# --------------------------------------------------------------------------
# case builders — each returns (manifest_case, files dict name → text bytes)
# --------------------------------------------------------------------------

def check_case(name, old_blocks, new_blocks, aligned, moved):
    """Validate a hand-labeled case before writing the manifest."""
    assert len(set(aligned)) == len(aligned), f"{name}: duplicate pairs"
    olds = [o for o, _ in aligned]
    news = [n for _, n in aligned]
    assert len(set(olds)) == len(olds), f"{name}: old side not one-to-one"
    assert len(set(news)) == len(news), f"{name}: new side not one-to-one"
    for o, n in aligned:
        assert 0 <= o < len(old_blocks), f"{name}: old index {o} out of range"
        assert 0 <= n < len(new_blocks), f"{name}: new index {n} out of range"
    for o, n in moved:
        assert (o, n) in aligned, f"{name}: moved pair {(o, n)} not in aligned"
        assert abs(o - n) >= 2, f"{name}: moved pair {(o, n)} displacement < 2"
        assert old_blocks[o] == new_blocks[n], f"{name}: moved pair {(o, n)} text differs"
    # Sanity: identical-text pairs at displacement >= 2 must be in moved.
    for o, n in aligned:
        if old_blocks[o] == new_blocks[n] and abs(o - n) >= 2:
            assert (o, n) in moved, \
                f"{name}: equal pair {(o, n)} displaced >=2 missing from moved"
    print(f"case {name}: {len(old_blocks)} old / {len(new_blocks)} new blocks, "
          f"{len(aligned)} gold pairs ({len(moved)} moved)")


def txt_case(name, old_md, new_md, aligned, moved):
    files = {f"{name}_old.txt": old_md, f"{name}_new.txt": new_md}
    old_blocks, new_blocks = txt_blocks(old_md), txt_blocks(new_md)
    check_case(name, old_blocks, new_blocks, aligned, moved)
    return (
        {
            "name": name,
            "old": f"{name}_old.txt",
            "new": f"{name}_new.txt",
            "old_blocks": old_blocks,
            "new_blocks": new_blocks,
            "aligned": aligned,
            "moved": moved,
        },
        files,
    )


def docx_case(name, old_paras, new_paras, aligned, moved):
    old_blocks, new_blocks = docx_blocks(old_paras), docx_blocks(new_paras)
    check_case(name, old_blocks, new_blocks, aligned, moved)
    return (
        {
            "name": name,
            "old": f"{name}_old.docx",
            "new": f"{name}_new.docx",
            "old_blocks": old_blocks,
            "new_blocks": new_blocks,
            "aligned": aligned,
            "moved": moved,
        },
        None,  # docx files written separately below
    )


# --------------------------------------------------------------------------
# case 1: EN TXT contract — anchors, modified, added, removed, one far move
# --------------------------------------------------------------------------

CONTRACT_EN_OLD = """# Service Agreement

Introductory paragraph describing the purpose.

## Article 1 Definitions

The service is provided as-is.

Support is available on business days.

## Article 2 Payment Terms

The total price is 12.500.000 VND.

Payment within 30 days of the invoice date.

## Article 3 Confidentiality

Both parties keep information secret.

## Article 4 Liability

Liability is capped at 1,234,567 USD.
"""

CONTRACT_EN_NEW = """# Service Agreement

Introductory paragraph describing the purpose.

## Article 1 Definitions

The service is provided as-is with updates.

## Article 2 Payment Terms

The total price is 15.750.000 VND.

Payment within 45 days of the invoice date.

Renewal happens automatically each year.

## Article 3 Confidentiality

Both parties keep information secret.

Support is available on business days.

## Article 5 Governing Law

Disputes follow Vietnamese law courts.
"""


def case_contract_en_txt():
    return txt_case(
        "contract_en_txt",
        CONTRACT_EN_OLD,
        CONTRACT_EN_NEW,
        aligned=[(0, 0), (1, 1), (2, 2), (3, 3), (4, 10), (5, 4), (6, 5),
                 (7, 6), (8, 8), (9, 9)],
        moved=[(4, 10)],
    )


# --------------------------------------------------------------------------
# case 2: VI TXT contract — Điều/Phụ lục clause anchors, money anchor,
# appendix moved to the front (far moves), added paragraph
# --------------------------------------------------------------------------

CONTRACT_VI_OLD = """# Hợp đồng dịch vụ

Văn bản này quy định quyền và nghĩa vụ của hai bên.

## Điều 1: Tổng quát

Hai bên cam kết thực hiện đúng các điều khoản.

Hội thoại hỗ trợ kỹ thuật diễn ra trong giờ hành chính.

## Điều 2: Thanh toán

Tổng giá trị hợp đồng là 12.500.000 đồng.

Thanh toán trong vòng 30 ngày kể từ ngày nhận hóa đơn.

## Điều 3: Bảo mật

Các bên phải bảo mật thông tin của hợp đồng.

## Phụ lục A: Danh sách dịch vụ

Chi tiết các dịch vụ được liệt kê bên dưới.
"""

CONTRACT_VI_NEW = """# Hợp đồng dịch vụ

Văn bản này quy định quyền và nghĩa vụ của các bên ký kết.

## Phụ lục A: Danh sách dịch vụ

Chi tiết các dịch vụ được liệt kê bên dưới.

## Điều 1: Tổng quát

Hai bên cam kết thực hiện đầy đủ các điều khoản đã ký.

Hội thoại hỗ trợ kỹ thuật diễn ra trong giờ hành chính.

## Điều 2: Thanh toán

Tổng giá trị hợp đồng là 12.500.000 đồng.

Thanh toán trong vòng 45 ngày kể từ ngày nhận hóa đơn.

## Điều 3: Bảo mật

Các bên phải bảo mật tuyệt đối thông tin hợp đồng.

Phụ lục có hiệu lực cùng ngày với hợp đồng chính.
"""


def case_contract_vi_txt():
    return txt_case(
        "contract_vi_txt",
        CONTRACT_VI_OLD,
        CONTRACT_VI_NEW,
        aligned=[(0, 0), (1, 1), (2, 4), (3, 5), (4, 6), (5, 7), (6, 8),
                 (7, 9), (8, 10), (9, 11), (10, 2), (11, 3)],
        # Moved = identical text at a displaced ordinal. The modified pairs
        # (3,5), (7,9), (9,11) are displaced too, but diff reports them as
        # Modified, not Moved.
        moved=[(2, 4), (4, 6), (5, 7), (6, 8), (8, 10), (10, 2), (11, 3)],
    )


# --------------------------------------------------------------------------
# case 3: EN DOCX report — modified paragraphs, added block, no moves
# --------------------------------------------------------------------------

REPORT_EN_OLD = [
    ("Heading1", "Quarterly Report"),
    (None, "Revenue grew in the third quarter of the fiscal year."),
    ("Heading1", "Financial Highlights"),
    (None, "Operating costs totaled 8.750.000 EUR in the reporting period."),
    (None, "Customer retention improved to ninety percent."),
    ("Heading1", "Outlook"),
    (None, "Next quarter focuses on enterprise customers."),
]

REPORT_EN_NEW = [
    ("Heading1", "Quarterly Report"),
    (None, "Revenue grew strongly in the third quarter of the fiscal year."),
    ("Heading1", "Financial Highlights"),
    (None, "Operating costs totaled 9.120.000 EUR in the reporting period."),
    (None, "Customer retention improved to ninety two percent."),
    (None, "Churn decreased across all market segments."),
    ("Heading1", "Outlook"),
    (None, "Next quarter focuses on enterprise customers and partners."),
]


def case_report_en_docx():
    return docx_case(
        "report_en_docx",
        REPORT_EN_OLD,
        REPORT_EN_NEW,
        aligned=[(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 6), (6, 7)],
        moved=[],
    )


# --------------------------------------------------------------------------
# case 4: VI DOCX report — money anchor, tail block relocated (moved)
# --------------------------------------------------------------------------

REPORT_VI_OLD = [
    ("Heading1", "Báo cáo quý"),
    (None, "Doanh thu tăng trưởng ổn định trong ba tháng vừa qua."),
    ("Heading1", "Kết quả tài chính"),
    (None, "Chi phí vận hành đạt 2.450.000 đồng trong kỳ báo cáo."),
    (None, "Số lượng khách hàng mới tăng đều mỗi tháng."),
    ("Heading1", "Định hướng kỳ tới"),
    (None, "Ưu tiên phát triển kênh khách hàng doanh nghiệp."),
]

REPORT_VI_NEW = [
    ("Heading1", "Báo cáo quý"),
    (None, "Doanh thu tăng trưởng ổn định trong ba tháng vừa qua."),
    ("Heading1", "Kết quả tài chính"),
    (None, "Chi phí vận hành đạt 2.450.000 đồng trong kỳ báo cáo."),
    ("Heading1", "Định hướng kỳ tới"),
    (None, "Ưu tiên phát triển kênh khách hàng doanh nghiệp."),
    (None, "Số lượng khách hàng mới tăng đều mỗi tháng."),
    (None, "Tỉ lệ giữ chân khách hàng đạt mức cao nhất từ trước tới nay."),
]


def case_report_vi_docx():
    return docx_case(
        "report_vi_docx",
        REPORT_VI_OLD,
        REPORT_VI_NEW,
        aligned=[(0, 0), (1, 1), (2, 2), (3, 3), (4, 6), (5, 4), (6, 5)],
        moved=[(4, 6)],
    )


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------

def main():
    os.makedirs(GOLD, exist_ok=True)

    cases = []
    files = {}
    for builder in (case_contract_en_txt, case_contract_vi_txt):
        case, txt_files = builder()
        cases.append(case)
        files.update(txt_files)

    docx_sets = {
        "report_en_docx_old.docx": REPORT_EN_OLD,
        "report_en_docx_new.docx": REPORT_EN_NEW,
        "report_vi_docx_old.docx": REPORT_VI_OLD,
        "report_vi_docx_new.docx": REPORT_VI_NEW,
    }
    for builder in (case_report_en_docx, case_report_vi_docx):
        case, _ = builder()
        cases.append(case)

    for fname, text in files.items():
        with open(os.path.join(GOLD, fname), "w", encoding="utf-8") as f:
            f.write(text)
        print("wrote", fname, len(text.encode()), "bytes")
    for fname, paras in docx_sets.items():
        docx_write(os.path.join(GOLD, fname), paras)
        print("wrote", fname, os.path.getsize(os.path.join(GOLD, fname)), "bytes")

    manifest = {
        "config": {
            "similarity_threshold": 0.55,
            "gap_penalty": -0.2,
            "heading_gap_weight": 1.5,
            "move_window": 3,
            "move_similarity_threshold": 0.85,
            "max_chunk": 2000,
        },
        "gates": {
            # Real e5-small embedder (the phase-03 quality gate).
            "real": {"precision_min": 0.85, "recall_min": 0.80},
            # Deterministic fake embedder (default CI). Lower documented bar
            # than the real gate: this corpus is lexical-heavy so the fake
            # scores above the real model here (actuals on 2026-11-18:
            # P 1.000 / R 1.000, move P 1.000 / R 1.000); the bar still
            # catches alignment regressions in default CI.
            "fake": {"precision_min": 0.80, "recall_min": 0.75},
        },
        "cases": cases,
    }
    with open(os.path.join(GOLD, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)
        f.write("\n")
    total_pairs = sum(len(c["aligned"]) for c in cases)
    total_moved = sum(len(c["moved"]) for c in cases)
    print(f"manifest written: {len(cases)} cases, {total_pairs} gold pairs "
          f"({total_moved} moved)")


if __name__ == "__main__":
    main()
