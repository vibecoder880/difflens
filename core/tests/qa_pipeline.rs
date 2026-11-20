//! QA hands-on integration tests (v1.0.2 release hardening, 2026-11-19).
//!
//! Real end-to-end invocations of the full pipeline over generated document
//! pairs: extract → align (real e5-small embedder when built with `--features
//! embeddings-ort`) → diff → heuristic enrich, plus store/report roundtrips,
//! the download-manager checksum path and a real llama-server LLM roundtrip.
//!
//! The documents are hermetic: on first use the harness runs the committed,
//! stdlib-only generator `scripts/gen-fixtures/gen_qa_docs.py` (python3)
//! which writes the pairs into the workspace `target/qa-docs/` directory and
//! drops a `.generated` marker. Delete the directory to force a fresh
//! generation; no network, no model files, no `/tmp` state involved.
//!
//! Run (PDF tests need the staged pdfium, set below):
//! ```text
//! DIFFLENS_PDFIUM_PATH=resources/binaries/pdfium/lib/libpdfium.so \
//!   cargo test -p difflens-core --features embeddings-ort \
//!   --test qa_pipeline -- --nocapture --test-threads=1
//! cargo test -p difflens-core --features embeddings-ort --test qa_pipeline \
//!   qa_llm_real_roundtrip_vi -- --ignored --nocapture --test-threads=1
//! ```

use std::path::PathBuf;
use std::sync::Once;

use difflens_core::model::{BlockKind, ChangeKind, Severity, SpanOp};
use difflens_core::pipeline;
use difflens_core::store::{NewSession, Store};

/// Workspace directory holding the generated QA document pairs.
fn qa_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/qa-docs")
}

/// Generate the QA document set once per process when the `.generated`
/// marker is absent (first run, or after `rm -rf target/qa-docs`).
fn generate_qa_docs() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let dir = qa_dir();
        if dir.join(".generated").is_file() {
            return;
        }
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/gen-fixtures/gen_qa_docs.py");
        assert!(
            script.is_file(),
            "QA fixture generator missing at {}",
            script.display()
        );
        std::fs::create_dir_all(&dir).expect("create qa-docs dir");
        let status = std::process::Command::new("python3")
            .arg("-I")
            .arg(&script)
            .arg(&dir)
            .status()
            .expect(
                "python3 is required to generate the QA fixtures — install it \
                 or run scripts/gen-fixtures/gen_qa_docs.py manually",
            );
        assert!(
            status.success(),
            "QA fixture generation failed ({status}); delete {} and retry",
            dir.display()
        );
        assert!(
            dir.join(".generated").is_file(),
            "generator ran but wrote no .generated marker into {}",
            dir.display()
        );
    });
}

fn doc(name: &str) -> String {
    generate_qa_docs();
    let p = qa_dir().join(name);
    assert!(
        p.is_file(),
        "missing QA document {} — run: python3 -I scripts/gen-fixtures/gen_qa_docs.py",
        p.display()
    );
    p.to_str().unwrap().to_string()
}

/// Point PDFium at the repo-staged copy once per process (only PDF tests
/// need it; every other test ignores the variable).
fn ensure_pdfium() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let staged = manifest
            .join("..")
            .join("resources")
            .join("binaries")
            .join("pdfium")
            .join("lib")
            .join("libpdfium.so");
        // SAFETY: test process, single-threaded init via Once; no other test
        // in this binary reads or writes this variable.
        unsafe {
            std::env::set_var("DIFFLENS_PDFIUM_PATH", &staged);
        }
        assert!(
            staged.is_file(),
            "staged pdfium missing at {}",
            staged.display()
        );
    });
}

/// The pipeline's default embedder is the REAL e5-small when the
/// `embeddings-ort` feature is on; every pipeline test asserts
/// `embeddings_used` so a silent fake fallback fails loudly.
fn run_pipeline(old: &str, new: &str) -> difflens_core::model::DiffResult {
    pipeline::run(old, new, pipeline::default_embedder().as_ref()).expect("pipeline run")
}

#[cfg(feature = "embeddings-ort")]
fn assert_real_embeddings(result: &difflens_core::model::DiffResult) {
    assert!(
        result.embeddings_used,
        "expected the real e5-small embedder; got the fake fallback \
         (is resources/models/e5-small present?)"
    );
}

#[cfg(not(feature = "embeddings-ort"))]
fn assert_real_embeddings(result: &difflens_core::model::DiffResult) {
    eprintln!("NOTE: built without embeddings-ort; fake embedder in use");
    let _ = result;
}

/// First change whose text, spans or flag evidence touches `needle`.
/// (Char-refined spans carry only the differing digits, so flag values are
/// often the only place the full `0,075%`-style string survives.)
fn find_change<'a>(
    result: &'a difflens_core::model::DiffResult,
    needle: &str,
) -> &'a difflens_core::model::Change {
    result
        .changes
        .iter()
        .find(|c| {
            c.old_text.as_deref().unwrap_or_default().contains(needle)
                || c.new_text.as_deref().unwrap_or_default().contains(needle)
                || c.spans.iter().any(|s| s.text.contains(needle))
                || c.flags.iter().any(|f| {
                    f.old_value.as_deref().unwrap_or_default().contains(needle)
                        || f.new_value.as_deref().unwrap_or_default().contains(needle)
                })
        })
        .unwrap_or_else(|| panic!("no change mentioning {needle:?}"))
}

fn flag<'a>(
    change: &'a difflens_core::model::Change,
    rule: &str,
) -> &'a difflens_core::model::HeuristicFlag {
    change
        .flags
        .iter()
        .find(|f| f.rule_id == rule)
        .unwrap_or_else(|| panic!("rule {rule} not fired for change {}", change.id))
}

// ===========================================================================
// Test 1 — Vietnamese contract pair (DOCX)
// ===========================================================================

#[test]
fn qa_docx_contract_full_pipeline() {
    let result = run_pipeline(&doc("contract-v1.docx"), &doc("contract-v2.docx"));
    assert_real_embeddings(&result);
    assert_eq!(result.format, difflens_core::model::DocFormat::Docx);

    // Moved pairs are detected as Moved — not mass delete + insert.
    let stats = &result.stats;
    assert_eq!(stats.moved, 2, "stats: {stats:?}");
    assert_eq!(stats.added, 1, "stats: {stats:?}");
    assert_eq!(stats.removed, 1, "stats: {stats:?}");
    assert_eq!(stats.modified, 3, "stats: {stats:?}");
    assert_eq!(stats.equal, 11, "stats: {stats:?}");

    let moved: Vec<&difflens_core::model::Change> = result
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Moved)
        .collect();
    assert_eq!(moved.len(), 2);
    for m in &moved {
        // Identical content repositioned: one Equal span, no delete/insert soup.
        assert_eq!(m.spans.len(), 1);
        assert_eq!(m.spans[0].op, SpanOp::Equal);
        assert!(m.similarity.unwrap() > 0.99);
    }
    assert!(
        moved
            .iter()
            .any(|m| m.spans[0].text.contains("bàn giao phần mềm")),
        "the delivery clause move was not detected"
    );
    assert!(
        moved
            .iter()
            .any(|m| m.spans[0].text.contains("bảo hành sản phẩm")),
        "the warranty clause move was not detected"
    );

    // Payment change: duration 30 → 15 days with delta −0.5, percent 0,05% → 0,075%.
    let payment = find_change(&result, "0,075%");
    assert_eq!(payment.kind, ChangeKind::Modified);
    let dur = flag(payment, "DURATION_CHANGED");
    assert!(
        (dur.delta.unwrap() - (-0.5)).abs() < 1e-9,
        "DURATION_CHANGED delta = {:?}",
        dur.delta
    );
    assert!(
        dur.old_value.as_deref().unwrap().contains("30 ngày"),
        "old {:?}",
        dur.old_value
    );
    assert!(
        dur.new_value.as_deref().unwrap().contains("15 ngày"),
        "new {:?}",
        dur.new_value
    );
    let pct = flag(payment, "PERCENT_CHANGED");
    assert!(
        (pct.delta.unwrap() - 0.5).abs() < 1e-9,
        "percent delta {:?}",
        pct.delta
    );

    // Penalty-money change: 5.000.000₫ → 12.000.000₫, severity ≥ High.
    let penalty = find_change(&result, "12.000.000₫");
    assert_eq!(penalty.kind, ChangeKind::Modified);
    let money = flag(penalty, "MONEY_CHANGED");
    assert!(
        (money.delta.unwrap() - 1.4).abs() < 1e-9,
        "money delta {:?}",
        money.delta
    );
    let sev = penalty.severity.expect("enriched severity");
    assert!(sev >= Severity::High, "penalty change severity = {sev:?}");

    // Deleted clause and added confidentiality clause.
    let removed = result
        .changes
        .iter()
        .find(|c| {
            c.kind == ChangeKind::Removed
                && c.old_text
                    .as_deref()
                    .unwrap_or_default()
                    .contains("báo cáo tiến độ")
        })
        .expect("deleted clause reported as Removed");
    assert!(removed.new_text.is_none() && removed.new_block_ids.is_empty());
    let added = result
        .changes
        .iter()
        .find(|c| {
            c.kind == ChangeKind::Added
                && c.new_text
                    .as_deref()
                    .unwrap_or_default()
                    .contains("Bảo mật thông tin")
        })
        .expect("new confidentiality clause reported as Added");
    assert!(added.old_text.is_none());
    assert!(
        added.flags.iter().any(|f| f.rule_id == "ENTITY_ADDED"),
        "the 3-year duration entity in the added clause should be flagged"
    );

    // Heading rename surfaced as a Modified heading change with risk keywords.
    let heading = result
        .changes
        .iter()
        .find(|c| c.spans.iter().any(|s| s.text.contains("bảo mật")))
        .expect("renamed heading change present");
    assert_eq!(heading.kind, ChangeKind::Modified);
    assert!(heading.severity.is_some());
    assert!(
        heading.flags.iter().any(|f| f.rule_id == "KEYWORD_RISK"),
        "heading rename should carry the KEYWORD_RISK flag"
    );

    // Audit trail: one entry per non-Equal change, in change order.
    let non_equal = result
        .changes
        .iter()
        .filter(|c| c.kind != ChangeKind::Equal)
        .count();
    assert_eq!(result.audit.len(), non_equal, "audit trail size mismatch");
    for (a, c) in result.audit.iter().zip(
        result
            .changes
            .iter()
            .filter(|c| c.kind != ChangeKind::Equal),
    ) {
        assert_eq!(a.change_id, c.id);
        assert_eq!(a.severity, c.severity.unwrap());
        assert!(
            !a.rules.is_empty() || a.severity == Severity::Info,
            "change {} has neither rules nor Info severity",
            c.id
        );
    }

    // Determinism: run twice → byte-identical JSON (includes the audit trail).
    let again = run_pipeline(&doc("contract-v1.docx"), &doc("contract-v2.docx"));
    let j1 = serde_json::to_string(&result).unwrap();
    let j2 = serde_json::to_string(&again).unwrap();
    assert_eq!(j1, j2, "pipeline output is not deterministic");
}

// ===========================================================================
// Test 2 — PDF pair (page numbers + alignment over pdfium blocks)
// ===========================================================================

#[test]
fn qa_pdf_pair_pages_and_alignment() {
    // The generator skips the PDF fixtures when no Vietnamese-capable TTF
    // exists (rare: CI and dev VMs ship DejaVu). Skip instead of failing.
    if qa_dir().join(".pdf-font-missing").is_file() {
        eprintln!("SKIP: no Vietnamese-capable TTF found — PDF fixtures not generated");
        return;
    }
    ensure_pdfium();
    let result = run_pipeline(&doc("contract-v1.pdf"), &doc("contract-v2.pdf"));
    assert_real_embeddings(&result);
    assert_eq!(result.format, difflens_core::model::DocFormat::Pdf);
    assert!(
        result.changes.iter().any(|c| !c.spans.is_empty()),
        "no inline spans — pdfium text extraction likely failed"
    );

    // Page numbers are present on every paired change and stay within the
    // real page count of the extracted PDFs.
    let v1 = difflens_core::extract::extract_pdf(&doc("contract-v1.pdf")).expect("extract v1 pdf");
    let pages = v1.page_count.expect("pdf page count");
    assert!(
        pages >= 2,
        "contract PDF should paginate, got {pages} page(s)"
    );
    let paired = result
        .changes
        .iter()
        .filter(|c| c.kind != ChangeKind::Added && c.kind != ChangeKind::Removed)
        .collect::<Vec<_>>();
    assert!(paired.len() >= 10, "unexpectedly few paired changes");
    for c in &paired {
        assert!(c.old_page.is_some(), "change {} missing old_page", c.id);
        assert!(c.new_page.is_some(), "change {} missing new_page", c.id);
        assert!(c.old_page.unwrap() >= 1 && c.old_page.unwrap() <= pages);
        assert!(c.new_page.unwrap() >= 1 && c.new_page.unwrap() <= pages);
    }
    // Both pages carry blocks (real pagination, not everything on page 1).
    assert!(
        v1.blocks.iter().any(|b| b.page == Some(pages)),
        "last page has no blocks"
    );

    // Same contract deltas must fire through the PDF path.
    let payment = find_change(&result, "0,075%");
    flag(payment, "DURATION_CHANGED");
    flag(payment, "PERCENT_CHANGED");
    let penalty = find_change(&result, "12.000.000₫");
    let money = flag(penalty, "MONEY_CHANGED");
    assert!((money.delta.unwrap() - 1.4).abs() < 1e-9);

    // Extraction fidelity: Vietnamese diacritics survived pdfium.
    let payment = find_change(&result, "0,075%");
    let joined: String = payment.spans.iter().map(|s| s.text.as_str()).collect();
    assert!(joined.contains("Thanh toán"), "text: {joined:?}");
    assert!(
        joined.contains("kể từ ngày xuất hóa đơn"),
        "text: {joined:?}"
    );
    // The percent digits survive as char-refined spans ("0,0" + "7" + "5%…").
    assert!(joined.contains("0,0"), "text: {joined:?}");
}

// ===========================================================================
// Test 3 — XLSX pair (Cell blocks + money delta)
// ===========================================================================

#[test]
fn qa_xlsx_cell_money_delta() {
    // Extraction level: every block is a Cell with sheet/row/col coordinates.
    let v1 = difflens_core::extract::extract_xlsx(&doc("price-v1.xlsx")).expect("extract xlsx");
    assert!(v1.blocks.len() >= 10, "blocks: {}", v1.blocks.len());
    for b in &v1.blocks {
        assert!(
            matches!(b.kind, BlockKind::Cell { .. }),
            "expected Cell block, got {:?}",
            b.kind
        );
    }
    let b2 = &v1.blocks[4]; // row 1, col 1 → "1.500.000₫"
    let BlockKind::Cell { sheet, row, col } = &b2.kind else {
        panic!("not a cell");
    };
    assert_eq!(sheet, "Bảng giá");
    assert_eq!((*row, *col), (1, 1));
    assert_eq!(b2.text, "1.500.000₫");

    let result = run_pipeline(&doc("price-v1.xlsx"), &doc("price-v2.xlsx"));
    assert_real_embeddings(&result);
    assert_eq!(result.format, difflens_core::model::DocFormat::Xlsx);
    assert!(result.stats.equal >= 9, "stats: {:?}", result.stats);
    assert_eq!(result.stats.modified, 3, "stats: {:?}", result.stats);

    // Money change on that very cell: 1.500.000₫ → 2.000.000₫, delta +1/3.
    let money_change = result
        .changes
        .iter()
        .find(|c| c.flags.iter().any(|f| f.rule_id == "MONEY_CHANGED"))
        .expect("MONEY_CHANGED did not fire");
    assert_eq!(money_change.old_block_ids, vec![v1.blocks[4].id.clone()]);
    let money = flag(money_change, "MONEY_CHANGED");
    assert!(
        (money.delta.unwrap() - 1.0 / 3.0).abs() < 1e-9,
        "delta {:?}",
        money.delta
    );
    assert_eq!(money.old_value.as_deref(), Some("1.500.000₫"));
    assert_eq!(money.new_value.as_deref(), Some("2.000.000₫"));
    assert_eq!(money_change.old_page, None, "spreadsheets have no pages");

    // Reference change 2026/HD-001 → 2026/HD-002.
    let ref_change = result
        .changes
        .iter()
        .find(|c| c.flags.iter().any(|f| f.rule_id == "REFERENCE_CHANGED"))
        .expect("REFERENCE_CHANGED did not fire");
    let refflag = flag(ref_change, "REFERENCE_CHANGED");
    assert!(
        refflag
            .old_value
            .as_deref()
            .unwrap()
            .contains("2026/HD-001")
    );
    assert!(
        refflag
            .new_value
            .as_deref()
            .unwrap()
            .contains("2026/HD-002")
    );

    // Plain text change: note cell gains the CPI clause (insertion).
    let text_change = find_change(&result, "CPI");
    assert_eq!(text_change.kind, ChangeKind::Modified);
}

// ===========================================================================
// Test 4 — PPTX pair (Slide blocks)
// ===========================================================================

#[test]
fn qa_pptx_slide_diff() {
    // Extraction level: Slide blocks with 0-based index + 1-based page.
    let v1 = difflens_core::extract::extract_pptx(&doc("deck-v1.pptx")).expect("extract pptx");
    assert_eq!(v1.blocks.len(), 3, "blocks: {}", v1.blocks.len());
    for (i, b) in v1.blocks.iter().enumerate() {
        let BlockKind::Slide { index } = &b.kind else {
            panic!("expected Slide block, got {:?}", b.kind);
        };
        assert_eq!(*index, i as u32);
        assert_eq!(b.page, Some(i as u32 + 1));
    }
    assert_eq!(v1.blocks[0].anchor.as_deref(), Some("Hợp đồng dịch vụ"));

    let result = run_pipeline(&doc("deck-v1.pptx"), &doc("deck-v2.pptx"));
    assert_real_embeddings(&result);
    assert_eq!(result.format, difflens_core::model::DocFormat::Pptx);
    assert_eq!(result.stats.equal, 1, "stats: {:?}", result.stats);
    assert!(result.stats.modified >= 2, "stats: {:?}", result.stats);

    // Slide 1: 6 tháng → 4 tháng (duration delta −1/3).
    let slide1 = find_change(&result, "Thời gian thực hiện");
    assert_eq!(slide1.kind, ChangeKind::Modified);
    let dur = flag(slide1, "DURATION_CHANGED");
    assert!(
        (dur.delta.unwrap() - (-1.0 / 3.0)).abs() < 1e-9,
        "delta {:?}",
        dur.delta
    );
    assert_eq!(slide1.old_page, Some(1));
    assert_eq!(slide1.new_page, Some(1));

    // Slide 2 gained the "Giai đoạn 3" milestone line.
    let slide2 = find_change(&result, "Kiểm thử");
    assert_eq!(slide2.kind, ChangeKind::Modified);
    assert_eq!(slide2.old_page, Some(2));
}

// ===========================================================================
// Test 5 — TXT/Markdown pair (word-level span sanity + hostile block)
// ===========================================================================

#[test]
fn qa_md_word_spans() {
    let result = run_pipeline(&doc("contract-md-v1.md"), &doc("contract-md-v2.md"));
    assert_eq!(result.format, difflens_core::model::DocFormat::Txt);

    // The payment paragraph changed 30 → 45 days (paragraph = the two
    // consecutive non-blank lines of Điều 1, joined by the extractor).
    let payment = find_change(&result, "45");
    assert_eq!(payment.kind, ChangeKind::Modified);

    let old_expected = "Bên B thanh toán trong vòng 30 ngày kể từ ngày xuất hóa đơn. \
Mọi khoản thanh toán chuyển khoản vào tài khoản Vietcombank chi nhánh Quận 1.";
    let new_expected = old_expected.replace("30 ngày", "45 ngày");

    // Spans reconstruct BOTH sides exactly (they cover the whole texts).
    let rebuilt_old: String = payment
        .spans
        .iter()
        .filter(|s| s.op != SpanOp::Insert)
        .map(|s| s.text.as_str())
        .collect();
    let rebuilt_new: String = payment
        .spans
        .iter()
        .filter(|s| s.op != SpanOp::Delete)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(
        rebuilt_old, old_expected,
        "spans do not reconstruct the old side"
    );
    assert_eq!(
        rebuilt_new, new_expected,
        "spans do not reconstruct the new side"
    );

    // The changed words only: delete/insert content is small, not a rewrite.
    let deleted: String = payment
        .spans
        .iter()
        .filter(|s| s.op == SpanOp::Delete)
        .map(|s| s.text.as_str())
        .collect();
    let inserted: String = payment
        .spans
        .iter()
        .filter(|s| s.op == SpanOp::Insert)
        .map(|s| s.text.as_str())
        .collect();
    assert!(deleted.contains("30"), "deleted: {deleted:?}");
    assert!(inserted.contains("45"), "inserted: {inserted:?}");
    assert!(
        deleted.chars().count() <= 10 && inserted.chars().count() <= 10,
        "spans cover more than the changed words: del={deleted:?} ins={inserted:?}"
    );
    // No whole-document rewrite span: several equal runs must remain.
    let equal_runs = payment
        .spans
        .iter()
        .filter(|s| s.op == SpanOp::Equal)
        .count();
    assert!(
        equal_runs >= 2,
        "expected multiple equal runs, got {equal_runs}"
    );
    let longest_equal = payment
        .spans
        .iter()
        .filter(|s| s.op == SpanOp::Equal)
        .map(|s| s.text.chars().count())
        .max()
        .unwrap_or(0);
    assert!(
        longest_equal < old_expected.chars().count(),
        "one Equal span swallows the whole document"
    );

    // Hostile added clause is carried verbatim for the report tests.
    let hostile = result
        .changes
        .iter()
        .find(|c| {
            c.new_text
                .as_deref()
                .unwrap_or_default()
                .contains("<script>alert('dl-xss')</script>")
        })
        .expect("hostile added clause present");
    assert!(
        hostile
            .new_text
            .as_deref()
            .unwrap()
            .contains("```fence thủ công```")
    );
}

// ===========================================================================
// Test 7 — Store + reports roundtrip (HTML escaping, deltas, md fences)
// ===========================================================================

#[test]
fn qa_store_and_reports_roundtrip() {
    let docx_result = run_pipeline(&doc("contract-v1.docx"), &doc("contract-v2.docx"));
    let md_result = run_pipeline(&doc("contract-md-v1.md"), &doc("contract-md-v2.md"));
    let xlsx_result = run_pipeline(&doc("price-v1.xlsx"), &doc("price-v2.xlsx"));

    // --- Store: insert → list → get → byte-identical result JSON ---
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("difflens-qa.sqlite3");
    let store = Store::open(&db_path).expect("open store");
    let result_json = serde_json::to_string(&docx_result).unwrap();
    store
        .insert_session(&NewSession {
            id: "job-qa-docx".into(),
            created_at: 1_700_000_000_000,
            old_path: docx_result.old_path.clone(),
            new_path: docx_result.new_path.clone(),
            format: docx_result.format,
            stats: docx_result.stats.clone(),
            embeddings_used: docx_result.embeddings_used,
            ocr_used: docx_result.ocr_used,
            result_json: result_json.clone(),
        })
        .expect("insert session");

    let listed = store.list_sessions(10, 0).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "job-qa-docx");
    assert_eq!(listed[0].format, difflens_core::model::DocFormat::Docx);
    assert_eq!(listed[0].stats.moved, 2);

    let got = store
        .get_session("job-qa-docx")
        .expect("get")
        .expect("present");
    assert_eq!(got.created_at, 1_700_000_000_000);
    let restored = got.result.expect("stored result deserializes");
    assert_eq!(restored.stats, docx_result.stats);
    assert_eq!(
        serde_json::to_string(&restored).unwrap(),
        result_json,
        "stored result JSON is not byte-stable through the store roundtrip"
    );

    // --- Reports ---
    fn input(r: &difflens_core::model::DiffResult) -> difflens_core::report::ReportInput<'_> {
        difflens_core::report::ReportInput {
            id: "job-qa-docx",
            result: r,
            created_at: 1_700_000_000_000,
            lang: difflens_core::report::ReportLang::En,
            version: "1.0.2-qa",
        }
    }

    let html = difflens_core::report::html::render_html(&input(&docx_result));
    let html_md = difflens_core::report::html::render_html(&input(&md_result));
    let html_xlsx = difflens_core::report::html::render_html(&input(&xlsx_result));
    let md = difflens_core::report::markdown::render_markdown(&input(&md_result));

    // Persist for the external HTML-parser verification step (before the
    // asserts so artifacts survive a failure).
    std::fs::write(qa_dir().join("report.html"), &html_md).unwrap();
    std::fs::write(qa_dir().join("report-contract.html"), &html).unwrap();
    std::fs::write(qa_dir().join("report-xlsx.html"), &html_xlsx).unwrap();
    std::fs::write(qa_dir().join("report.md"), &md).unwrap();

    // Content preserved: the raw Vietnamese percent survives escaping.
    assert!(html.contains("0,075%"), "percent value lost in HTML report");
    assert!(html.contains("Thanh toán"));
    // Delta formatting (U+2212 minus, signed percent) present for each rule.
    assert!(html.contains("−50%"), "duration delta formatting missing");
    assert!(html.contains("+140%"), "money delta formatting missing");
    assert!(html.contains("+50%"), "percent delta formatting missing");
    assert!(
        !html.contains("<script"),
        "HTML report must never contain a raw <script"
    );

    assert!(
        html_md.contains("&lt;script&gt;alert(&#x27;dl-xss&#x27;)&lt;/script&gt;"),
        "hostile markup not escaped"
    );
    assert!(html_md.contains("&lt;img src=x onerror=alert(2)&gt;"));
    assert!(!html_md.contains("<script"), "raw script tag leaked");

    assert!(
        html_xlsx.contains("+33.3%"),
        "xlsx money delta formatting missing"
    );

    // Markdown: fences chosen longer than any backtick run in the content.
    assert!(
        md.contains("```fence thủ công```"),
        "content backticks lost"
    );
    // The hostile line must sit inside a fence widened to 4 (content has a
    // 3-backtick run) so the run cannot terminate the block early.
    let lines: Vec<&str> = md.lines().collect();
    let hostile_line = lines
        .iter()
        .position(|l| l.contains("<script>alert"))
        .expect("hostile text in md");
    let fence = lines[..hostile_line]
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty() && l.chars().all(|c| c == '`'))
        .expect("fence line above hostile content");
    assert_eq!(fence.chars().count(), 4, "fence not widened: {fence:?}");
    assert!(md.contains("0,1%/ngày"));
}

// ===========================================================================
// Test 6b — download manager checksum verification path (real HTTP, loopback)
// ===========================================================================

/// Tiny loopback HTTP server serving a fixed body (accepts repeated
/// connections; dies with the test process).
fn serve_bytes(body: &'static [u8]) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    addr
}

#[test]
fn qa_download_manager_checksum_path() {
    use difflens_core::download::{DownloadSpec, download_file};

    const BODY: &[u8] = b"diffens-qa-download-payload-0123456789";
    let url = format!("http://{}/qa.bin", serve_bytes(BODY));
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("qa.bin");

    let expected = difflens_core::download::checksum::sha256_bytes(BODY);
    let spec = DownloadSpec::new(url.clone(), dest.clone(), expected.clone());
    let path = download_file(&spec, &mut |_, _| true).expect("valid checksum download");
    assert_eq!(path, dest);
    assert_eq!(std::fs::read(&dest).unwrap(), BODY);

    // Wrong digest → download fails, `.part` removed, destination untouched.
    let bad = DownloadSpec::new(url, dir.path().join("bad.bin"), "00".repeat(32));
    let err = download_file(&bad, &mut |_, _| true).unwrap_err();
    assert!(
        err.to_string().contains("checksum mismatch"),
        "unexpected error: {err}"
    );
    assert!(!dir.path().join("bad.bin").exists());
}

// ===========================================================================
// Test 6 — real LLM roundtrip (ignored; needs llama-server + GGUF)
// ===========================================================================

#[test]
#[ignore = "needs llama-server binary + 639MB GGUF; run: cargo test -p difflens-core --features embeddings-ort --test qa_pipeline qa_llm_real_roundtrip_vi -- --ignored --nocapture"]
fn qa_llm_real_roundtrip_vi() {
    use difflens_core::download::manifest::LlmManifest;
    use difflens_core::download::{DownloadSpec, download_file};
    use difflens_core::llm::client::ChatOptions;
    use difflens_core::llm::explain::{
        ContextBudget, Lang, build_explanation_messages, change_context_from, parse_explanation,
    };
    use difflens_core::llm::sidecar::{SidecarConfig, SidecarProcess};

    let manifest = LlmManifest::embedded();
    let model = manifest
        .model("qwen3-0.6b-q8_0-test")
        .expect("manifest lost its test model entry");

    // Self-sufficient install through the download manager (same flow as
    // llm_tests::llm_real_roundtrip, shared DIFFLENS_LLM_CACHE-aware cache
    // root): download-if-missing, checksum-verify, hardened extraction.
    let cache = std::env::var_os("DIFFLENS_LLM_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/llm-cache"));
    let bin_root = cache.join("bin");
    let models_dir = cache.join("models");
    std::fs::create_dir_all(&bin_root).unwrap();
    std::fs::create_dir_all(&models_dir).unwrap();

    let server_asset = manifest
        .llama_server
        .assets
        .iter()
        .find(|a| a.target_triple == difflens_core::download::manifest::current_target_triple())
        .unwrap_or_else(|| panic!("no pinned llama-server asset for this triple in the manifest"));

    let archive_path = bin_root.join(format!(
        "llama-{}-bin-{}.archive",
        manifest.llama_server.tag, server_asset.id
    ));
    if !archive_path.exists() {
        let mut spec = DownloadSpec::new(
            server_asset.url.clone(),
            archive_path.clone(),
            server_asset.sha256.clone(),
        );
        spec.expected_bytes = Some(server_asset.bytes);
        download_file(&spec, &mut |_, _| true).expect("llama-server bundle download");
    }
    let server_bin = match difflens_core::llm::sidecar::detect_installed(
        &bin_root,
        &manifest.llama_server.tag,
    ) {
        Some(p) => p,
        None => difflens_core::llm::sidecar::install_bundle(
            &archive_path,
            server_asset.archive,
            &manifest.llama_server.tag,
            &bin_root,
            &difflens_core::download::archive::ExtractLimits::default(),
        )
        .expect("llama-server install"),
    };

    // Model file locations, newest-first; download through the manager if
    // none of the caches has it (checksum verified on every path).
    let model_path = if let Some(p) = std::env::var_os("DIFFLENS_QA_GGUF").map(PathBuf::from) {
        p
    } else {
        let shared = models_dir.join(&model.file);
        if shared.is_file() {
            shared
        } else {
            let mut spec =
                DownloadSpec::new(model.url.clone(), shared.clone(), model.sha256.clone());
            spec.expected_bytes = Some(model.bytes);
            download_file(&spec, &mut |_, _| true).expect("test GGUF download");
            shared
        }
    };

    // Checksum verification path against the manifest pin (real sha256 of
    // the whole 639 MB file).
    difflens_core::download::checksum::verify_file(&model_path, &model.sha256, 0)
        .expect("GGUF fails the manifest checksum");

    // The same contract pair as test 1, change #1 = the payment-term change.
    let result = run_pipeline(&doc("contract-v1.docx"), &doc("contract-v2.docx"));
    let change = result
        .changes
        .iter()
        .find(|c| c.flags.iter().any(|f| f.rule_id == "DURATION_CHANGED"))
        .expect("payment-term change present");
    let ctx = change_context_from(change);
    let messages = build_explanation_messages(&ctx, Lang::Vi, &ContextBudget::default());

    let mut cfg = SidecarConfig::new(server_bin, model_path);
    cfg.context_size = 2048;
    cfg.threads = Some(2); // 3-core VM: leave a core for the test process
    let started = std::time::Instant::now();
    let server = SidecarProcess::spawn(&cfg).expect("llama-server sidecar spawn");
    eprintln!(
        "sidecar ready at {} in {:?}",
        server.base_url,
        started.elapsed()
    );

    let client = server.client().expect("client");
    let opts = ChatOptions {
        model: "test".into(),
        max_tokens: 256,
        ..ChatOptions::default()
    };
    let t0 = std::time::Instant::now();
    let mut tokens: Vec<String> = Vec::new();
    let stream = client
        .chat_stream(&messages, &opts, &mut |delta: &str| {
            tokens.push(delta.to_string());
            true
        })
        .expect("real chat stream");
    let elapsed = t0.elapsed();
    eprintln!(
        "{} tokens in {:?} ({:.1} tok/s), finish_reason={:?}",
        tokens.len(),
        elapsed,
        tokens.len() as f64 / elapsed.as_secs_f64().max(0.001),
        stream.finish_reason
    );

    // Streamed tokens arrived and the stream terminated cleanly.
    assert!(tokens.len() >= 5, "only {} tokens streamed", tokens.len());
    assert!(
        matches!(
            stream.finish_reason.as_deref(),
            Some("stop") | Some("length")
        ),
        "unclean termination: {:?}",
        stream.finish_reason
    );
    let parsed = parse_explanation(&stream.text);
    eprintln!("summary: {}", parsed.summary);
    eprintln!("impact: {}", parsed.impact);
    assert!(!parsed.summary.trim().is_empty(), "empty summary");
    assert!(!parsed.impact.trim().is_empty(), "empty impact");

    // The explanation must plausibly reference the payment-term change.
    let combined = format!("{} {}", parsed.summary, parsed.impact).to_lowercase();
    let mentioned = ["15", "30", "ngày", "thanh toán", "thời hạn", "hạn", "khoản"]
        .iter()
        .any(|k| combined.contains(k));
    assert!(
        mentioned,
        "explanation does not mention the payment term: {combined:?}"
    );
    std::fs::write(qa_dir().join("explanation-vi.txt"), &stream.text).unwrap();

    server.kill();
}
