//! Golden tests for the HTML + Markdown report renderers (phase-08).
//!
//! Focus: the SEC1 hostile-content invariant — every document-derived
//! string (paths, spans, added/removed text, flag values, rule details) is
//! escaped in HTML output and kept inside intact fences in Markdown — plus
//! determinism, Vietnamese diacritics, long content and structural asserts
//! (contains/absent, not byte snapshots).

use difflens_core::model::{
    AuditEntry, Change, ChangeKind, DiffResult, DiffSpan, DiffStats, DocFormat, HeuristicFlag,
    Severity, SpanOp,
};
use difflens_core::report::{
    ReportInput, ReportLang, html::render_html, markdown::render_markdown,
};

/// Fixed timestamp so outputs are deterministic across runs.
const FIXED_MS: i64 = 1_795_005_296_789; // 2026-11-18 12:34:56.789 UTC
const VERSION: &str = "1.0.0";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// DiffResult whose text is entirely attacker-controlled XSS payloads.
fn hostile_result() -> DiffResult {
    let payload_script = "<script>alert('pwned')</script>";
    let payload_img = "<img src=x onerror=alert(1)>";
    let payload_rtl = "normal \u{202E}reversed\u{202C} text";
    let payload_ticks = "code ```run``` end";
    DiffResult {
        old_path: "/attacker/<script>old</script>.docx".into(),
        new_path: "/attacker/[\u{202E}]new\u{202C}.docx".into(),
        format: DocFormat::Docx,
        changes: vec![
            Change {
                id: "c0".into(),
                kind: ChangeKind::Modified,
                old_block_ids: vec!["a0".into()],
                new_block_ids: vec!["b0".into()],
                old_page: Some(1),
                new_page: Some(1),
                spans: vec![
                    DiffSpan {
                        op: SpanOp::Equal,
                        text: "Payment within ".into(),
                    },
                    DiffSpan {
                        op: SpanOp::Delete,
                        text: payload_script.to_string(),
                    },
                    DiffSpan {
                        op: SpanOp::Insert,
                        text: payload_img.to_string(),
                    },
                ],
                old_text: None,
                new_text: None,
                similarity: Some(0.5),
                move_delta: None,
                severity: Some(Severity::Critical),
                flags: vec![HeuristicFlag {
                    rule_id: "RULE_MONEY_CHANGED".into(),
                    label_key: "flag.moneyChanged".into(),
                    old_value: Some(payload_ticks.to_string()),
                    new_value: Some("1\u{202E}00$".into()),
                    delta: Some(-0.5),
                    detail: Some(payload_rtl.to_string()),
                }],
            },
            Change {
                id: "c1".into(),
                kind: ChangeKind::Added,
                old_block_ids: vec![],
                new_block_ids: vec!["b9".into()],
                old_page: None,
                new_page: Some(2),
                spans: vec![],
                old_text: None,
                new_text: Some(format!("{payload_script}\n{payload_img}")),
                similarity: None,
                move_delta: None,
                severity: Some(Severity::Medium),
                flags: vec![],
            },
            Change {
                id: "c2".into(),
                kind: ChangeKind::Removed,
                old_block_ids: vec!["a9".into()],
                new_block_ids: vec![],
                old_page: Some(2),
                new_page: None,
                spans: vec![],
                old_text: Some(payload_ticks.to_string()),
                new_text: None,
                similarity: None,
                move_delta: None,
                severity: Some(Severity::Low),
                flags: vec![],
            },
        ],
        stats: DiffStats {
            added: 1,
            removed: 1,
            modified: 1,
            moved: 0,
            equal: 10,
        },
        embeddings_used: false,
        ocr_used: true,
        audit: vec![AuditEntry {
            change_id: "c0".into(),
            severity: Severity::Critical,
            rules: vec![HeuristicFlag {
                rule_id: "RULE_MONEY_CHANGED".into(),
                label_key: "flag.moneyChanged".into(),
                old_value: Some(payload_script.to_string()),
                new_value: Some(payload_img.to_string()),
                delta: Some(-0.5),
                detail: None,
            }],
        }],
    }
}

fn input<'a>(result: &'a DiffResult, lang: ReportLang) -> ReportInput<'a> {
    ReportInput {
        id: "job-7",
        result,
        created_at: FIXED_MS,
        lang,
        version: VERSION,
    }
}

fn clean_result() -> DiffResult {
    // Vietnamese contract-style text with diacritics + currency symbols.
    let old = "Điều khoản 5: Tiền thuê là 12.000.000₫/tháng, thanh toán trong 30 ngày.";
    let new = "Điều khoản 5: Tiền thuê là 15.500.000₫/tháng, thanh toán trong 45 ngày.";
    DiffResult {
        old_path: "/hk/hợp đồng cũ.docx".into(),
        new_path: "/hk/hợp đồng mới.docx".into(),
        format: DocFormat::Docx,
        changes: vec![Change {
            id: "c0".into(),
            kind: ChangeKind::Modified,
            old_block_ids: vec!["a0".into()],
            new_block_ids: vec!["b0".into()],
            old_page: Some(1),
            new_page: Some(1),
            spans: vec![
                DiffSpan {
                    op: SpanOp::Equal,
                    text: "Điều khoản 5: Tiền thuê là ".into(),
                },
                DiffSpan {
                    op: SpanOp::Delete,
                    text: old["Điều khoản 5: Tiền thuê là ".len()..].to_string(),
                },
                DiffSpan {
                    op: SpanOp::Insert,
                    text: new["Điều khoản 5: Tiền thuê là ".len()..].to_string(),
                },
            ],
            old_text: None,
            new_text: None,
            similarity: Some(0.92),
            move_delta: None,
            severity: Some(Severity::High),
            flags: vec![HeuristicFlag {
                rule_id: "RULE_MONEY_CHANGED".into(),
                label_key: "flag.moneyChanged".into(),
                old_value: Some("12.000.000₫".into()),
                new_value: Some("15.500.000₫".into()),
                delta: Some(0.2917),
                detail: None,
            }],
        }],
        stats: DiffStats {
            added: 0,
            removed: 0,
            modified: 1,
            moved: 0,
            equal: 12,
        },
        embeddings_used: true,
        ocr_used: false,
        audit: vec![AuditEntry {
            change_id: "c0".into(),
            severity: Severity::High,
            rules: vec![HeuristicFlag {
                rule_id: "RULE_MONEY_CHANGED".into(),
                label_key: "flag.moneyChanged".into(),
                old_value: Some("12.000.000₫".into()),
                new_value: Some("15.500.000₫".into()),
                delta: Some(0.2917),
                detail: None,
            }],
        }],
    }
}

fn long_result() -> DiffResult {
    let long_text = format!("{}{}{}", "A".repeat(2500), "B".repeat(2500), "END");
    DiffResult {
        old_path: "/tmp/long-old.txt".into(),
        new_path: "/tmp/long-new.txt".into(),
        format: DocFormat::Txt,
        changes: vec![Change {
            id: "c0".into(),
            kind: ChangeKind::Added,
            old_block_ids: vec![],
            new_block_ids: vec!["b0".into()],
            old_page: None,
            new_page: None,
            spans: vec![],
            old_text: None,
            new_text: Some(long_text),
            similarity: None,
            move_delta: None,
            severity: None,
            flags: vec![],
        }],
        stats: DiffStats::default(),
        embeddings_used: false,
        ocr_used: false,
        audit: vec![],
    }
}

// ---------------------------------------------------------------------------
// Hostile content: HTML
// ---------------------------------------------------------------------------

#[test]
fn hostile_html_escapes_every_document_derived_string() {
    let html = render_html(&input(&hostile_result(), ReportLang::En));
    let doc_strings = [
        "<script>alert('pwned')</script>",
        "<img src=x onerror=alert(1)>",
        "/attacker/<script>old</script>.docx",
    ];
    for raw in doc_strings {
        assert!(!html.contains(raw), "raw payload must not survive: {raw:?}");
    }
    // Escaped renderings must be present instead.
    assert!(html.contains("&lt;script&gt;"), "escaped <script> expected");
    assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(html.contains("/attacker/&lt;script&gt;old&lt;/script&gt;.docx"));
    // RTL overrides are neutralized to numeric references.
    assert!(
        !html.contains('\u{202E}'),
        "raw RTL override must not survive"
    );
    assert!(html.contains("&#x202E;"));
    // The document itself must never open a script element.
    assert!(!html.contains("<script"));
    // Inline spans render escaped del/ins markup.
    assert!(html.contains("<del class=\"del\">&lt;script&gt;"));
    assert!(html.contains("<ins class=\"ins\">&lt;img"));
    // OCR note present (ocr_used = true).
    assert!(html.contains("Includes pages extracted via OCR"));
}

#[test]
fn hostile_html_keeps_structure_and_labels() {
    let html = render_html(&input(&hostile_result(), ReportLang::En));
    // Standalone: inline style, no external assets, no JS requirement.
    assert!(html.contains("<style>"));
    assert!(!html.contains("src=\"http"));
    assert!(!html.contains("href=\"http"));
    assert!(html.starts_with("<!doctype html>\n<html lang=\"en\">"));
    assert!(html.ends_with("</html>\n"));
    // Header content.
    assert!(html.contains("Document comparison report"));
    assert!(html.contains("2026-11-18 12:34:56.789 UTC"));
    // Summary counts.
    assert!(html.contains("stat-added"));
    // Change groups.
    assert!(html.contains("Modified (1)"));
    assert!(html.contains("Added (1)"));
    assert!(html.contains("Removed (1)"));
    // Severity badges.
    assert!(html.contains("sev-critical"));
    assert!(html.contains("sev-low"));
    // Audit trail section with the fired rule.
    assert!(html.contains("Audit trail"));
    assert!(html.contains("Money amount changed"));
    // Footer disclaimer + version.
    assert!(
        html.contains("Generated locally by DiffLens — AI-assisted review only, not legal advice.")
    );
    assert!(html.contains("DiffLens 1.0.0"));
}

#[test]
fn hostile_html_vi_labels_render() {
    let html = render_html(&input(&hostile_result(), ReportLang::Vi));
    assert!(html.starts_with("<!doctype html>\n<html lang=\"vi\">"));
    assert!(html.contains("Báo cáo so sánh tài liệu"));
    assert!(html.contains("Số tiền thay đổi"));
    assert!(html.contains(
        "Được tạo cục bộ bởi DiffLens — chỉ hỗ trợ rà soát bằng AI, không phải tư vấn pháp lý."
    ));
    // Payloads still escaped in VI output.
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script"));
}

// ---------------------------------------------------------------------------
// Hostile content: Markdown
// ---------------------------------------------------------------------------

#[test]
fn hostile_md_escapes_nothing_but_keeps_fence_integrity() {
    let md = render_markdown(&input(&hostile_result(), ReportLang::En));
    // Content survives raw inside fences (no HTML escaping in Markdown).
    // The Modified change renders its spans as -/+ lines.
    assert!(md.contains("- Payment within <script>alert('pwned')</script>"));
    assert!(md.contains("+ Payment within <img src=x onerror=alert(1)>"));
    // Fence integrity: the payload with a ``` run must be inside a fence
    // strictly longer than 3 backticks (payload run is 3 → fence = 4).
    assert!(
        md.contains("````\n- code ```run``` end\n````"),
        "fence: {md}"
    );
    // Table cells stay tables: pipes escaped, no raw newlines in cells.
    assert!(!md.contains("| Old file | /attacker/&lt;")); // no HTML escaping in md
    assert!(md.contains("| Old file | /attacker/<script>old</script>.docx |"));
    // Structure.
    assert!(md.contains("# Document comparison report"));
    assert!(md.contains("## Summary"));
    assert!(md.contains("## Changes"));
    assert!(md.contains("### Modified (1)"));
    assert!(md.contains("#### `c0` — Critical"));
    assert!(md.contains("## Audit trail"));
    assert!(md.contains("- Money amount changed — Old:"));
    // Footer.
    assert!(
        md.contains("Generated locally by DiffLens — AI-assisted review only, not legal advice.")
    );
    assert!(md.contains("DiffLens 1.0.0"));
    assert!(md.contains("2026-11-18 12:34:56.789 UTC"));
}

#[test]
fn hostile_md_table_cells_with_pipes_stay_intact() {
    let mut result = clean_result();
    result.changes[0].flags[0].old_value = Some("a | b".into());
    result.audit[0].rules[0].old_value = Some("a | b".into());
    let md = render_markdown(&input(&result, ReportLang::En));
    // Bullets are not tables — a raw pipe is safe there.
    assert!(md.contains("Old: `a | b`"), "bullet: {md}");
    // Audit table cells must escape the pipe to keep the table intact.
    assert!(
        md.contains("Old: a \\| b"),
        "audit cell must escape pipe: {md}"
    );
    // The table rows keep exactly 3 columns in the audit section.
    for line in md.lines().filter(|l| l.starts_with("| `c0` |")) {
        // Count delimiter pipes only (escaped `\|` cells are content).
        let delimiters = line.replace("\\|", "").matches('|').count();
        assert_eq!(delimiters, 4, "audit row must keep 4 delimiters: {line}");
    }
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn same_input_and_timestamp_render_byte_identical_reports() {
    let a = render_html(&input(&hostile_result(), ReportLang::En));
    let b = render_html(&input(&hostile_result(), ReportLang::En));
    assert_eq!(a, b);

    let md_a = render_markdown(&input(&hostile_result(), ReportLang::En));
    let md_b = render_markdown(&input(&hostile_result(), ReportLang::En));
    assert_eq!(md_a, md_b);
}

#[test]
fn different_timestamp_changes_only_timestamp_lines() {
    let a = render_html(&input(&hostile_result(), ReportLang::En));
    let result = hostile_result();
    let mut later = input(&result, ReportLang::En);
    later.created_at = FIXED_MS + 60_000;
    let b = render_html(&later);
    assert_ne!(a, b);
    assert!(b.contains("2026-11-18 12:35:56.789 UTC"));
}

// ---------------------------------------------------------------------------
// Vietnamese diacritics + long content
// ---------------------------------------------------------------------------

#[test]
fn vietnamese_diacritics_render_untouched() {
    let html = render_html(&input(&clean_result(), ReportLang::Vi));
    assert!(html.contains("Điều khoản 5: Tiền thuê là "));
    assert!(html.contains("12.000.000₫/tháng"));
    assert!(html.contains("15.500.000₫/tháng"));
    assert!(html.contains("Tệp cũ"));
    assert!(html.contains("hk/hợp đồng cũ.docx"));
    assert!(html.contains("+29.2%")); // delta +0.2917 → +29.2%

    let md = render_markdown(&input(&clean_result(), ReportLang::Vi));
    assert!(
        md.contains("+ Điều khoản 5: Tiền thuê là 15.500.000₫/tháng, thanh toán trong 45 ngày.")
    );
    assert!(
        md.contains("- Điều khoản 5: Tiền thuê là 12.000.000₫/tháng, thanh toán trong 30 ngày.")
    );
}

#[test]
fn long_content_renders_completely() {
    let html = render_html(&input(&long_result(), ReportLang::En));
    assert!(html.contains(&"A".repeat(200)));
    assert!(html.contains("BBB"));
    assert!(html.contains("END"));

    let md = render_markdown(&input(&long_result(), ReportLang::En));
    assert!(md.contains(&"A".repeat(200)));
    assert!(md.contains("BBBEND")); // tail of the single added line
}

// ---------------------------------------------------------------------------
// Empty result (documents match)
// ---------------------------------------------------------------------------

#[test]
fn empty_diff_renders_no_changes_section_content() {
    let result = DiffResult {
        old_path: "a.txt".into(),
        new_path: "b.txt".into(),
        format: DocFormat::Txt,
        changes: vec![],
        stats: DiffStats::default(),
        embeddings_used: false,
        ocr_used: false,
        audit: vec![],
    };
    let html = render_html(&input(&result, ReportLang::En));
    assert!(html.contains("No differences found — the documents match."));
    assert!(!html.contains("Audit trail"), "no audit section when empty");
    let md = render_markdown(&input(&result, ReportLang::En));
    assert!(md.contains("No differences found — the documents match."));
    assert!(!md.contains("## Audit trail"));
}
