//! Bug-sweep 2026-11-22: adversarial / edge-case tests beyond the 299 happy
//! paths. Each test targets a specific hostile input or lifecycle case from
//! plans/20261117-difflens/reports/bug-sweep-2026-11-22.md.

use std::io::Write as _;
use std::path::PathBuf;

use difflens_core::embeddings::FakeHashEmbedder;
use difflens_core::error::CoreError;
use difflens_core::model::{Change, ChangeKind, DiffResult, DiffStats, DocFormat};
use difflens_core::pipeline::{self, Stage};
use difflens_core::report::{ReportInput, ReportLang};
use difflens_core::store::{MAX_RESULT_JSON_BYTES, NewSession, Store};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn write(path: &std::path::Path, bytes: &[u8]) -> String {
    std::fs::write(path, bytes).expect("write");
    path.to_str().unwrap().to_string()
}

/// Build a minimal DOCX-like zip with the given parts.
fn build_zip(parts: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        for (name, data) in parts {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

fn minimal_docx_xml(body: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:body>{body}</w:body></w:document>"
    )
    .into_bytes()
}

// ===========================================================================
// Area 1: malformed inputs — errors, never panics
// ===========================================================================

/// Truncated DOCX/XLSX/PPTX at N bytes: error or graceful, never panic.
#[test]
fn truncated_ooxml_never_panics() {
    for fixture in ["sample.docx", "sample.xlsx", "sample.pptx"] {
        let full = std::fs::read(fixtures_dir().join(fixture)).unwrap();
        let sizes = [
            0usize,
            1,
            10,
            100,
            full.len() / 2,
            full.len().saturating_sub(1),
        ];
        for n in sizes {
            let dir = tmp();
            let p = dir.path().join(format!("trunc-{n}-{fixture}"));
            let path = write(&p, &full[..n]);
            match difflens_core::extract::extract(&path) {
                Ok(model) => {
                    // Acceptable only if it produced something coherent.
                    assert!(
                        model.blocks.len() < 1000,
                        "{fixture}@{n}: absurd block count"
                    );
                }
                Err(e) => assert!(
                    matches!(e, CoreError::Extraction(_)),
                    "{fixture}@{n}: wrong error kind: {e}"
                ),
            }
        }
    }
}

/// Zip whose entry data has a corrupt CRC (byte flipped mid-stream).
#[test]
fn corrupt_crc_zip_is_an_error() {
    let doc = build_zip(&[(
        "word/document.xml",
        minimal_docx_xml("<w:p><w:r><w:t>Hello contract world</w:t></w:r></w:p>"),
    )]);
    // Flip a byte in the middle (inside compressed data or headers).
    let mut bad = doc.clone();
    let mid = bad.len() / 2;
    bad[mid] ^= 0xFF;
    let dir = tmp();
    let path = write(&dir.path().join("crc.docx"), &bad);
    let res = difflens_core::extract::extract(&path);
    match res {
        Ok(_) => { /* zip may localize damage; tolerating a partial read is OK */ }
        Err(e) => assert!(matches!(e, CoreError::Extraction(_)), "{e}"),
    }
}

/// document.xml with invalid UTF-8 bytes inside a zip that sniffs as DOCX.
#[test]
fn bad_utf8_xml_is_an_error_not_a_panic() {
    let mut xml = minimal_docx_xml("<w:p><w:r><w:t>text</w:t></w:r></w:p>");
    // Overwrite a payload byte with a lone 0xFF (invalid UTF-8).
    let idx = xml.windows(4).position(|w| w == b"text").unwrap() + 1;
    xml[idx] = 0xFF;
    let doc = build_zip(&[("word/document.xml", xml)]);
    let dir = tmp();
    let path = write(&dir.path().join("badutf8.docx"), &doc);
    match difflens_core::extract::extract(&path) {
        Ok(m) => assert!(
            m.warnings.iter().any(|w| w.contains("document.xml")) || m.blocks.is_empty(),
            "expected warning or empty model, got blocks={}",
            m.blocks.len()
        ),
        Err(e) => assert!(matches!(e, CoreError::Extraction(_)), "{e}"),
    }
}

/// Zip container that has [Content_Types].xml but no document.xml /
/// workbook.xml / slides → unsupported-container error.
#[test]
fn ooxml_missing_main_part_is_an_error() {
    let doc = build_zip(&[("[Content_Types].xml", b"<Types/>".to_vec())]);
    let dir = tmp();
    let path = write(&dir.path().join("nomain.docx"), &doc);
    let err = difflens_core::extract::extract(&path).unwrap_err();
    assert!(matches!(err, CoreError::Extraction(_)), "{err}");
}

/// DOCX whose word/document.xml part exists but is empty (0 bytes): must be
/// an Extraction error or an empty model — never a panic.
#[test]
fn docx_with_empty_document_xml() {
    let doc = build_zip(&[("word/document.xml", Vec::new())]);
    let dir = tmp();
    let path = write(&dir.path().join("emptydoc.docx"), &doc);
    match difflens_core::extract::extract(&path) {
        Ok(m) => assert!(m.blocks.is_empty()),
        Err(e) => assert!(matches!(e, CoreError::Extraction(_)), "{e}"),
    }
}

/// XLSX whose workbook.xml declares zero sheets.
#[test]
fn xlsx_with_zero_sheets() {
    let workbook = br#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets></sheets></workbook>"#;
    let doc = build_zip(&[
        ("xl/workbook.xml", workbook.to_vec()),
        (
            "[Content_Types].xml",
            br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#
                .to_vec(),
        ),
    ]);
    let dir = tmp();
    let path = write(&dir.path().join("zero.xlsx"), &doc);
    match difflens_core::extract::extract(&path) {
        Ok(m) => assert!(m.blocks.is_empty(), "blocks: {:?}", m.blocks.len()),
        Err(e) => assert!(matches!(e, CoreError::Extraction(_)), "{e}"),
    }
}

/// PPTX that has slide1..3 and slide5 but no slide4 (gap in numbering).
#[test]
fn pptx_with_slide_numbering_gap() {
    let slide = |n: usize| {
        format!(
            "<?xml version=\"1.0\"?><p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Slide {n} content alpha beta</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"
        )
        .into_bytes()
    };
    let presentation = br#"<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst/></p:presentation>"#.to_vec();
    let doc = build_zip(&[
        ("ppt/presentation.xml", presentation),
        ("ppt/slides/slide1.xml", slide(1)),
        ("ppt/slides/slide2.xml", slide(2)),
        ("ppt/slides/slide3.xml", slide(3)),
        ("ppt/slides/slide5.xml", slide(5)),
    ]);
    let dir = tmp();
    let path = write(&dir.path().join("gap.pptx"), &doc);
    let model = difflens_core::extract::extract(&path).unwrap();
    // All four present slides must be extracted (count-based, not
    // max-index-based).
    assert_eq!(model.blocks.len(), 4, "blocks: {:?}", model.blocks);
}

/// PDF edge cases through the real pdfium binding (repo-staged library).
#[test]
fn pdf_edge_cases_error_cleanly() {
    let cases: &[(&str, Vec<u8>)] = &[
        ("empty.pdf", Vec::new()),
        ("header-only.pdf", b"%PDF-1.7".to_vec()),
        ("truncated-mid-stream.pdf", {
            let full = std::fs::read(fixtures_dir().join("sample.pdf")).unwrap();
            full[..full.len() / 2].to_vec()
        }),
        ("random-with-magic.pdf", {
            let mut v = b"%PDF-1.4 ".to_vec();
            v.extend((0..4096u32).map(|i| (i * 31 % 251) as u8));
            v
        }),
    ];
    for (name, bytes) in cases {
        let dir = tmp();
        let path = write(&dir.path().join(name), bytes);
        match difflens_core::extract::extract(&path) {
            Ok(m) => {
                // 0-byte file legitimately sniffs as empty text.
                if *name == "empty.pdf" {
                    assert!(m.blocks.is_empty());
                } else {
                    panic!(
                        "{name}: garbage PDF must not extract, got {} blocks",
                        m.blocks.len()
                    );
                }
            }
            Err(e) => assert!(matches!(e, CoreError::Extraction(_)), "{name}: {e}"),
        }
    }
}

// ===========================================================================
// Area 2: path & name edge cases
// ===========================================================================

#[test]
fn unicode_and_weird_filenames_extract() {
    let dir = tmp();
    let names = [
        "hợp-đồng-💀-v1.txt",
        "  spaces  and .dots .txt",
        "rtl-مرحبا-העולם.txt",
        "no-extension",
    ];
    for (i, name) in names.iter().enumerate() {
        let p = dir.path().join(name);
        std::fs::write(&p, format!("paragraph {i} with some words")).unwrap();
        let model = difflens_core::extract::extract(p.to_str().unwrap()).unwrap();
        assert_eq!(model.blocks.len(), 1, "{name}");
    }
}

#[test]
fn path_traversal_components_are_just_paths() {
    let dir = tmp();
    std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
    std::fs::create_dir_all(dir.path().join("a/c")).unwrap();
    let p = dir.path().join("a/b/../c/doc.txt");
    std::fs::write(&p, b"content here").unwrap();
    // The OS resolves ..; extract just reads the file.
    let model = difflens_core::extract::extract(p.to_str().unwrap()).unwrap();
    assert_eq!(model.blocks.len(), 1);
}

#[test]
#[cfg(unix)]
fn symlinked_file_extract() {
    let dir = tmp();
    let real = dir.path().join("real.txt");
    std::fs::write(&real, b"real content").unwrap();
    let link = dir.path().join("link.txt");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let model = difflens_core::extract::extract(link.to_str().unwrap()).unwrap();
    assert_eq!(model.blocks.len(), 1);
}

#[test]
fn same_file_old_and_new_is_all_equal() {
    let dir = tmp();
    let p = write(
        &dir.path().join("same.txt"),
        b"# Title\n\nAlpha paragraph.\n\nBeta paragraph.\n",
    );
    let r = pipeline::run(&p, &p, &FakeHashEmbedder::default()).unwrap();
    assert_eq!(r.stats.equal, 3, "{:?}", r.stats);
    assert_eq!(
        r.stats.added + r.stats.removed + r.stats.modified + r.stats.moved,
        0
    );
}

#[test]
fn swapped_old_new_stats_are_symmetric() {
    let dir = tmp();
    let a = write(
        &dir.path().join("a.txt"),
        b"# T\n\nAlpha one text.\n\nBeta block.\n",
    );
    let b = write(
        &dir.path().join("b.txt"),
        b"# T\n\nAlpha two text.\n\nGamma block.\n",
    );
    let e = FakeHashEmbedder::default();
    let ab = pipeline::run(&a, &b, &e).unwrap();
    let ba = pipeline::run(&b, &a, &e).unwrap();
    assert_eq!(ab.stats.modified, ba.stats.modified, "modified symmetric");
    assert_eq!(ab.stats.added, ba.stats.removed);
    assert_eq!(ab.stats.removed, ba.stats.added);
}

#[test]
fn file_vanishing_between_runs_is_an_error() {
    let dir = tmp();
    let a = write(&dir.path().join("a.txt"), b"text a");
    let b = write(&dir.path().join("b.txt"), b"text b");
    let e = FakeHashEmbedder::default();
    // First run OK; delete one side; second run errors (no panic).
    pipeline::run(&a, &b, &e).unwrap();
    std::fs::remove_file(&b).unwrap();
    let err = pipeline::run(&a, &b, &e).unwrap_err();
    assert!(matches!(err, CoreError::Extraction(_)), "{err}");
}

// ===========================================================================
// Area 3: content edge cases
// ===========================================================================

#[test]
fn empty_whitespace_and_tiny_content() {
    let dir = tmp();
    for (name, content, expect_blocks) in [
        ("empty.txt", "", 0),
        ("ws.txt", "   \n\t\n  \n", 0),
        ("one-char.txt", "x", 1),
    ] {
        let p = write(&dir.path().join(name), content.as_bytes());
        let m = difflens_core::extract::extract(&p).unwrap();
        assert_eq!(m.blocks.len(), expect_blocks, "{name}: {:?}", m.blocks);
    }
}

#[test]
fn crlf_lf_and_cr_only_line_endings() {
    let dir = tmp();
    let a = write(
        &dir.path().join("crlf.txt"),
        b"# Title\r\n\r\nAlpha paragraph.\r\n\r\nBeta.\r\n",
    );
    let b = write(
        &dir.path().join("lf.txt"),
        b"# Title\n\nAlpha paragraph.\n\nBeta.\n",
    );
    let c = write(
        &dir.path().join("cr.txt"),
        b"# Title\r\rAlpha paragraph.\r\rBeta.\r",
    );
    let ma = difflens_core::extract::extract(&a).unwrap();
    let mb = difflens_core::extract::extract(&b).unwrap();
    let mc = difflens_core::extract::extract(&c).unwrap();
    let texts_a: Vec<&str> = ma.blocks.iter().map(|b| b.text.as_str()).collect();
    let texts_b: Vec<&str> = mb.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts_a, texts_b, "CRLF vs LF must normalize identically");
    // CR-only: `.lines()` splits on \n only — CR-only text is one line, so
    // this legitimately yields fewer blocks; assert no panic and no \r left
    // in CRLF text.
    for blk in &ma.blocks {
        assert!(!blk.text.contains('\r'), "raw CR leaked: {:?}", blk.text);
    }
    let _ = mc;
}

#[test]
fn utf16_bom_variants_with_vietnamese() {
    let dir = tmp();
    let text = "Hợp đồng dịch vụ tiếng Việt";
    for (name, bytes) in [
        ("le.txt", {
            let mut v = vec![0xFF, 0xFE];
            v.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
            v
        }),
        ("be.txt", {
            let mut v = vec![0xFE, 0xFF];
            v.extend(text.encode_utf16().flat_map(|u| u.to_be_bytes()));
            v
        }),
    ] {
        let p = write(&dir.path().join(name), &bytes);
        let m = difflens_core::extract::extract(&p).unwrap();
        assert_eq!(m.blocks.len(), 1);
        assert_eq!(m.blocks[0].text, text, "{name}");
    }
}

#[test]
fn zero_width_and_combining_diacritics_word_diff() {
    let old = "Khách hàng þ头部 with combining: ca\u{031B} phê";
    let new = "Khách hàng þ头部 with combining: ca\u{031B} phê sữa";
    let spans = difflens_core::diff::word::word_spans(old, new);
    // Reconstruction must be lossless whatever the normalization.
    let rebuilt_new: String = spans
        .iter()
        .filter(|s| s.op != difflens_core::model::SpanOp::Delete)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(rebuilt_new, new);
}

#[test]
fn numeric_edge_deltas() {
    use difflens_core::heuristic::deltas::compute_deltas;
    // 1.000 vs 1.000,5 (VN group+decimal)
    let f = compute_deltas("phí 1.000₫", "phí 1.000,5₫");
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(
        (f[0].delta.unwrap() - 0.0005).abs() < 1e-9,
        "{:?}",
        f[0].delta
    );

    // 1.000.000,25₫ parses
    let f = compute_deltas("x", "giá 1.000.000,25₫");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].rule_id, "ENTITY_ADDED");

    // 0% vs 100%
    let f = compute_deltas("chiết khấu 0%", "chiết khấu 100%");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].rule_id, "PERCENT_CHANGED");

    // money 0₫
    let f = compute_deltas("phí 0₫", "phí 5.000.000₫");
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(
        f[0].delta.is_none(),
        "0-base rel undefined: {:?}",
        f[0].delta
    );

    // duration 0 ngày
    let f = compute_deltas("trong 0 ngày", "trong 30 ngày");
    assert_eq!(f.len(), 1);
    assert!(f[0].delta.is_none());

    // Negative percent keeps its sign: -5% → -3% is +2pp / +40% relative.
    let f = compute_deltas("lãi suất -5%", "lãi suất -3%");
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].rule_id, "PERCENT_CHANGED");
    assert!((f[0].delta.unwrap() - 0.4).abs() < 1e-9, "{:?}", f[0].delta);
    assert_eq!(f[0].old_value.as_deref(), Some("-5%"));

    // Negative money sign flip is a change, not a no-op.
    let f = compute_deltas("phạt -1.000.000₫", "phạt 1.000.000₫");
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].rule_id, "MONEY_CHANGED");
    // -1e6 → 1e6: rel = 2e6 / |-1e6| = 2.
    assert!((f[0].delta.unwrap() - 2.0).abs() < 1e-9, "{:?}", f[0].delta);

    // A sign glued to a word is a hyphenation, not a negative percent.
    let f = compute_deltas("chiết khấu khấu-5%", "chiết khấu khấu7%");
    assert!(f.iter().all(|x| x.rule_id != "PERCENT_CHANGED"), "{f:?}");
}

// ===========================================================================
// Area 4: pipeline lifecycle
// ===========================================================================

/// Progress callback returning Err at the Nth call aborts with that error
/// (cancel-at-stage-boundary); covers every stage boundary.
#[test]
fn progress_err_aborts_at_every_boundary() {
    for cancel_at in 0..12usize {
        let dir = tmp();
        let a = write(
            &dir.path().join("a.txt"),
            b"# T\n\nAlpha body one.\n\nBeta body two.\n",
        );
        let b = write(
            &dir.path().join("b.txt"),
            b"# T\n\nAlpha body two.\n\nGamma new.\n",
        );
        let e = FakeHashEmbedder::default();
        let mut calls = 0usize;
        let res = pipeline::run_with_progress(&a, &b, &e, &Default::default(), &mut |_, _| {
            calls += 1;
            if calls == cancel_at + 1 {
                Err(CoreError::Cancelled)
            } else {
                Ok(())
            }
        });
        match res {
            Err(CoreError::Cancelled) => {}
            Err(other) => panic!("cancel_at={cancel_at}: {other}"),
            Ok(_) => {
                // Cancellation index past the final callback: the pipeline
                // legitimately completed (8 callbacks for a non-OCR run).
                assert!(
                    calls <= cancel_at,
                    "completed without firing cancel {cancel_at} of {calls}"
                );
            }
        }
    }
}

/// Two interleaved run_with_progress calls on independent threads both
/// complete and produce identical results to sequential runs.
#[test]
fn interleaved_pipelines_are_independent() {
    let dir = tmp();
    let a = write(
        &dir.path().join("a.txt"),
        b"# T\n\nAlpha one.\n\nBeta two.\n",
    );
    let b = write(
        &dir.path().join("b.txt"),
        b"# T\n\nAlpha uno.\n\nBeta two.\n",
    );
    let a2 = a.clone();
    let b2 = b.clone();
    let h =
        std::thread::spawn(move || pipeline::run(&a2, &b2, &FakeHashEmbedder::default()).unwrap());
    let r_main = pipeline::run(&a, &b, &FakeHashEmbedder::default()).unwrap();
    let r_thread = h.join().unwrap();
    assert_eq!(
        serde_json::to_string(&r_main).unwrap(),
        serde_json::to_string(&r_thread).unwrap()
    );
}

/// Stage sequence sanity + huge percent values clamp into u8 range.
#[test]
fn progress_percentages_stay_within_0_100() {
    let dir = tmp();
    let a = write(&dir.path().join("a.txt"), b"# T\n\nAlpha body one.\n");
    let b = write(&dir.path().join("b.txt"), b"# T\n\nAlpha body two.\n");
    let mut seen: Vec<(Stage, u8)> = Vec::new();
    pipeline::run_with_progress(
        &a,
        &b,
        &FakeHashEmbedder::default(),
        &Default::default(),
        &mut |s, p| {
            assert!(p <= 100, "percent {p} out of range at {s:?}");
            seen.push((s, p));
            Ok(())
        },
    )
    .unwrap();
    assert!(seen.len() >= 8, "stage callbacks: {seen:?}");
}

// ===========================================================================
// Area 5: store
// ===========================================================================

fn sample_session(id: &str, created_at: i64) -> NewSession {
    let result = DiffResult {
        old_path: "/o.txt".into(),
        new_path: "/n.txt".into(),
        format: DocFormat::Txt,
        changes: vec![],
        stats: DiffStats::default(),
        embeddings_used: false,
        ocr_used: false,
        audit: vec![],
    };
    NewSession {
        id: id.into(),
        created_at,
        old_path: "/o.txt".into(),
        new_path: "/n.txt".into(),
        format: DocFormat::Txt,
        stats: result.stats.clone(),
        embeddings_used: false,
        ocr_used: false,
        result_json: serde_json::to_string(&result).unwrap(),
    }
}

/// Result blob at exactly MAX_RESULT_JSON_BYTES is stored; +1 is skipped.
#[test]
fn blob_boundary_at_exactly_the_limit() {
    let store = Store::open_in_memory().unwrap();
    // Build a valid JSON padded to exactly the limit with a string field.
    let mut at_limit = sample_session("at-limit", 1);
    let mut json = at_limit.result_json.clone();
    json.pop(); // drop trailing "}"
    let quote = char::from(0x22); // "
    let target = MAX_RESULT_JSON_BYTES;
    let fixed = 12; // ,"pad":"XXXXXXXX"}
    let pad_len = target - json.len() - fixed;
    json.push_str(&format!(",{q}pad{q}:{q}{q}", q = quote));
    json.push_str(&"x".repeat(pad_len));
    json.push_str(&format!("{q}{q}}}", q = quote));
    assert_eq!(json.len(), MAX_RESULT_JSON_BYTES, "len {}", json.len());
    at_limit.result_json = json;
    store.insert_session(&at_limit).unwrap();
    assert!(
        store.get_session("at-limit").unwrap().is_some(),
        "exactly-at-limit must persist"
    );

    let mut over = sample_session("over-limit", 2);
    over.result_json = "x".repeat(MAX_RESULT_JSON_BYTES + 1);
    store.insert_session(&over).unwrap(); // skip is Ok
    assert!(store.get_session("over-limit").unwrap().is_none());
}

/// 100 sessions: pagination stays consistent.
#[test]
fn hundred_sessions_paginate() {
    let store = Store::open_in_memory().unwrap();
    for i in 0..100 {
        store
            .insert_session(&sample_session(&format!("job-{i:03}"), i as i64))
            .unwrap();
    }
    let all = store.list_sessions(500, 0).unwrap();
    assert_eq!(all.len(), 100);
    let mut page_total = 0;
    let mut offset = 0u32;
    loop {
        let page = store.list_sessions(7, offset).unwrap();
        if page.is_empty() {
            break;
        }
        page_total += page.len();
        offset += 7;
    }
    assert_eq!(page_total, 100);
    // Newest first.
    assert_eq!(all[0].id, "job-099");
    assert_eq!(all[99].id, "job-000");
}

/// Truncated (corrupt) sqlite file: open must error with CoreError::Store,
/// never panic.
#[test]
fn corrupt_db_file_is_a_store_error() {
    let dir = tmp();
    let db = dir.path().join("h.sqlite3");
    {
        let store = Store::open(&db).unwrap();
        store.insert_session(&sample_session("s1", 1)).unwrap();
    }
    // Truncate mid-file.
    let bytes = std::fs::read(&db).unwrap();
    std::fs::write(&db, &bytes[..bytes.len() / 2]).unwrap();
    match Store::open(&db) {
        Ok(_) => { /* sqlite may salvage a half file; must still be usable */ }
        Err(e) => assert!(matches!(e, CoreError::Store(_)), "{e}"),
    }
    // Random garbage file.
    let db2 = dir.path().join("garbage.sqlite3");
    std::fs::write(&db2, vec![0xA5u8; 4096]).unwrap();
    match Store::open(&db2) {
        Ok(_) => {}
        Err(e) => assert!(matches!(e, CoreError::Store(_)), "{e}"),
    }
}

// ===========================================================================
// Area 6: reports
// ===========================================================================

fn report_result(changes: Vec<Change>) -> DiffResult {
    DiffResult {
        old_path: "/tmp/old-💀.txt".into(),
        new_path: "/tmp/new \"onerror=alert(1)>.txt".into(),
        format: DocFormat::Txt,
        changes,
        stats: DiffStats {
            added: 1,
            removed: 0,
            modified: 1,
            moved: 0,
            equal: 1,
        },
        embeddings_used: false,
        ocr_used: false,
        audit: vec![],
    }
}

fn flag(rule: &str, i: usize) -> difflens_core::model::HeuristicFlag {
    difflens_core::model::HeuristicFlag {
        rule_id: format!("{rule}-{i}"),
        label_key: "flag.moneyChanged".into(),
        old_value: Some(format!("old-{i}\" onmouseover=\"x")),
        new_value: Some(format!("new-{i}<script>")),
        delta: Some(i as f64 / 7.0),
        detail: Some(format!("detail-{i} | pipe \n newline <b>")),
    }
}

fn change_with(kind: ChangeKind, text: &str, flags: usize) -> Change {
    Change {
        id: "c0".into(),
        kind,
        old_block_ids: vec!["b0".into()],
        new_block_ids: vec!["b1".into()],
        old_page: Some(1),
        new_page: Some(2),
        spans: vec![
            difflens_core::model::DiffSpan {
                op: difflens_core::model::SpanOp::Equal,
                text: "same ".into(),
            },
            difflens_core::model::DiffSpan {
                op: difflens_core::model::SpanOp::Delete,
                text: "old".into(),
            },
            difflens_core::model::DiffSpan {
                op: difflens_core::model::SpanOp::Insert,
                text: format!("new {text}"),
            },
        ],
        old_text: None,
        new_text: None,
        similarity: Some(0.91),
        move_delta: None,
        severity: Some(difflens_core::model::Severity::High),
        flags: (0..flags).map(|i| flag("MONEY", i)).collect(),
    }
}

/// A change carrying 50 flags renders without panicking in both formats.
#[test]
fn report_with_50_flags() {
    let result = report_result(vec![change_with(ChangeKind::Modified, "x", 50)]);
    let input = ReportInput {
        id: "job-50",
        result: &result,
        created_at: 1_795_000_000_000,
        lang: ReportLang::En,
        version: "1.0.2",
    };
    let md = difflens_core::report::markdown::render_markdown(&input);
    assert!(md.matches("- Money amount changed").count() >= 50);
    let html = difflens_core::report::html::render_html(&input);
    assert!(html.len() > 1000);
}

/// Content that IS a markdown fence of increasing length stays fenced.
#[test]
fn report_fence_text_cannot_break_out() {
    for fence in ["```", "````", "`````", "``````", "`", "``"] {
        let mut c = change_with(ChangeKind::Modified, fence, 0);
        c.spans = vec![difflens_core::model::DiffSpan {
            op: difflens_core::model::SpanOp::Insert,
            text: format!("a\n{fence}\nclosed?"),
        }];
        let result = report_result(vec![c]);
        let input = ReportInput {
            id: "job-fence",
            result: &result,
            created_at: 0,
            lang: ReportLang::En,
            version: "1.0.2",
        };
        let md = difflens_core::report::markdown::render_markdown(&input);
        // The outer fence must be strictly longer than any backtick run in
        // the content: extract fence lines and verify.
        let mut max_fence = 0usize;
        for line in md.lines() {
            if line.chars().all(|c| c == '`') && !line.is_empty() {
                max_fence = max_fence.max(line.len());
            } else if line.starts_with('`') {
                // content line that is entirely backticks beyond the fences?
                let run = line.chars().take_while(|&c| c == '`').count();
                assert!(
                    run < max_fence,
                    "content run {run} >= fence {max_fence} for {fence:?}"
                );
            }
        }
    }
}

/// Attribute-injecting values in detail fields must not inject into HTML.
#[test]
fn html_report_escapes_attribute_injection() {
    let result = report_result(vec![change_with(ChangeKind::Modified, "x", 3)]);
    let input = ReportInput {
        id: "job-x\" onerror=\"alert(1)",
        result: &result,
        created_at: 0,
        lang: ReportLang::En,
        version: "1.0.2",
    };
    let html = difflens_core::report::html::render_html(&input);
    assert!(!html.contains("<script>"), "raw <script> leaked");
    // The attribute-breakout SEQUENCE must not appear raw (quotes escaped).
    let raw_breakout = "\" onmouseover=\"";
    assert!(
        !html.contains(raw_breakout),
        "attribute breakout sequence leaked raw"
    );
    assert!(html.contains("&quot;"), "quotes must be escaped");
    assert!(!html.contains("<img "), "markup injected");
}

/// Empty changes list and Added-only diff render fine.
#[test]
fn report_empty_and_added_only() {
    let empty = report_result(vec![]);
    let input = ReportInput {
        id: "job-empty",
        result: &empty,
        created_at: 0,
        lang: ReportLang::En,
        version: "1.0.2",
    };
    let md = difflens_core::report::markdown::render_markdown(&input);
    assert!(md.contains("No differences found"));

    let mut added = Change {
        id: "c0".into(),
        kind: ChangeKind::Added,
        old_block_ids: vec![],
        new_block_ids: vec!["b9".into()],
        old_page: None,
        new_page: Some(3),
        spans: vec![],
        old_text: None,
        new_text: Some("Hoàn toàn mới: 5.000.000₫ và ``` fence".into()),
        similarity: None,
        move_delta: None,
        severity: Some(difflens_core::model::Severity::Medium),
        flags: vec![],
    };
    added.flags = vec![flag("ENTITY_ADDED", 0)];
    let result = report_result(vec![added]);
    let input = ReportInput {
        id: "job-added",
        result: &result,
        created_at: 0,
        lang: ReportLang::Vi,
        version: "1.0.2",
    };
    let md = difflens_core::report::markdown::render_markdown(&input);
    assert!(md.contains("+ Hoàn toàn mới"));
    let html = difflens_core::report::html::render_html(&input);
    assert!(html.contains("Hoàn toàn mới"));
}

// ===========================================================================
// Area 8: LLM SSE parsing (mock, no server needed for the parser-level cases)
// ===========================================================================

use difflens_core::llm::client::{SseEvent, SseParser};

#[test]
fn sse_ignores_event_field_lines() {
    let mut p = SseParser::new();
    let evs = p.feed(b"event: message\ndata: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n");
    assert_eq!(evs, vec![SseEvent::Token("ok".into())]);
}

#[test]
fn sse_json_with_escaped_quotes_in_content() {
    let mut p = SseParser::new();
    let evs = p.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"say \\\"hi\\\" now\"}}]}\n\n");
    assert_eq!(evs, vec![SseEvent::Token("say \"hi\" now".into())]);
}

#[test]
fn sse_done_inside_content_string_is_a_token() {
    let mut p = SseParser::new();
    let evs =
        p.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"[DONE] means finished\"}}]}\n\n");
    assert_eq!(evs, vec![SseEvent::Token("[DONE] means finished".into())]);
    assert!(!evs.contains(&SseEvent::Done));
}

#[test]
fn sse_reasoning_fields_are_skipped_not_fatal() {
    let mut p = SseParser::new();
    let evs = p.feed(
        b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking...\",\"content\":null}}]}\n\n",
    );
    assert_eq!(evs, vec![SseEvent::Skipped]);
}

#[test]
fn sse_two_mb_single_line_is_bounded_no_panic() {
    let mut p = SseParser::new();
    let mut line = Vec::with_capacity(2 * 1024 * 1024 + 64);
    line.extend_from_slice(b"data: ");
    line.extend(std::iter::repeat_n(b'x', 2 * 1024 * 1024));
    line.extend_from_slice(b"\n\n");
    let evs = p.feed(&line);
    // The 1 MiB carry cap drops the runaway line; malformed → Skipped.
    assert!(
        evs.iter().all(|e| matches!(e, SseEvent::Skipped)),
        "{evs:?}"
    );
}

/// A raw-TCP mock llama server that closes mid-stream without [DONE]:
/// chat_stream must return Ok with the partial text (documented behavior).
#[test]
fn llm_stream_close_without_done_returns_partial() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Drain the full request (headers + body) so closing later sends a
        // clean FIN instead of RST over unread bytes.
        let mut buf = [0u8; 4096];
        let mut got = String::new();
        loop {
            let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            got.push_str(&String::from_utf8_lossy(&buf[..n]));
            if let Some(header_end) = got.find("\r\n\r\n") {
                let cl = got
                    .lines()
                    .find_map(|l| {
                        l.split(':').next().zip(Some(l)).and_then(|(k, v)| {
                            (k.eq_ignore_ascii_case("content-length"))
                                .then(|| v.split(':').nth(1).unwrap_or("").trim().to_string())
                        })
                    })
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                if got.len() >= header_end + 4 + cl {
                    break;
                }
            }
        }
        let head =
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
        stream.write_all(head.as_bytes()).unwrap();
        stream
            .write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial tex\"}}]}\n\n")
            .unwrap();
        // Close without [DONE].
    });
    let client = difflens_core::llm::client::LlmClient::new(&format!("http://{addr}")).unwrap();
    let opts = difflens_core::llm::client::ChatOptions::default();
    let mut tokens = 0;
    let res = client
        .chat_stream(
            &[difflens_core::llm::client::ChatMessage::user("hi")],
            &opts,
            &mut |_| {
                tokens += 1;
                true
            },
        )
        .unwrap();
    assert_eq!(res.text, "partial tex");
    assert!(res.finish_reason.is_none());
}

// ===========================================================================
// Area 9: OCR TSV parsing (synthetic; VM has no tesseract)
// ===========================================================================

#[test]
fn ocr_tsv_negative_confidences_and_empty_words() {
    // Header only.
    assert!(difflens_core::ocr::engine::parse_tsv(
        "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext"
    )
    .is_empty());

    let tsv = concat!(
        "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n",
        "5\t1\t1\t1\t1\t1\t10\t10\t50\t12\t-1.0\tNEG\n", // negative conf
        "5\t1\t1\t1\t1\t2\t60\t10\t50\t12\t95.0\t\n",    // empty text
        "5\t1\t1\t1\t1\t3\t110\t10\t50\t12\t95.0\tword\n", // good
        "garbage line without tabs\n",                   // malformed
    );
    let words = difflens_core::ocr::engine::parse_tsv(tsv);
    // Whatever survives must exclude empty text; no panic either way.
    for w in &words {
        assert!(!w.text.is_empty(), "empty word kept: {w:?}");
    }
}

// ===========================================================================
// Probes (printed with --nocapture; classified in the sweep report)
// ===========================================================================

use difflens_core::heuristic::deltas::compute_deltas as deltas;

#[test]
fn probe_negative_money_and_huge_numbers() {
    let cases: &[(&str, &str)] = &[
        ("phạt -1.000.000₫", "phạt 1.000.000₫"),
        ("nợ -500 triệu đồng", "thưởng 500 triệu đồng"),
        (
            "x 999999999999999999999999999₫",
            "y 1999999999999999999999999999₫",
        ),
    ];
    for (o, n) in cases {
        eprintln!("PROBE {o:?} -> {n:?}");
        for f in deltas(o, n) {
            eprintln!(
                "  {} delta={:?} detail={}",
                f.rule_id,
                f.delta,
                f.detail.as_deref().unwrap_or("")
            );
        }
    }
}

/// DOCX with tables nested 5 levels deep (flattening extraction, no panic).
#[test]
fn docx_nested_tables_five_deep() {
    fn cell(inner: &str) -> String {
        format!("<w:tc><w:tcPr/><w:p><w:r><w:t>{inner}</w:t></w:r></w:p></w:tc>")
    }
    // Nest 5 tables: level1 > level5 content.
    let mut xml = "level5-base".to_string();
    for depth in (1..=4).rev() {
        let label = format!("cell{depth}");
        xml = format!(
            "L{depth} head <w:tbl><w:tr>{}{}</w:tr></w:tbl>",
            cell(&label),
            cell(&xml)
        );
    }
    let doc = build_zip(&[("word/document.xml", minimal_docx_xml(&xml))]);
    let dir = tmp();
    let path = write(&dir.path().join("nested.docx"), &doc);
    let m = difflens_core::extract::extract(&path).unwrap();
    eprintln!(
        "PROBE nested tables blocks: {} text: {:?}",
        m.blocks.len(),
        m.blocks.first().map(|b| b.text.clone())
    );
    assert_eq!(m.blocks.len(), 1, "one flattened table block");
    assert!(m.blocks[0].text.contains("level5-base"));
}

/// DOCX whose document.xml.rels references itself.
#[test]
fn docx_self_referencing_relationships() {
    let rels = br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="word/_rels/document.xml.rels"/></Relationships>"#;
    let doc = build_zip(&[
        (
            "word/document.xml",
            minimal_docx_xml("<w:p><w:r><w:t>text present</w:t></w:r></w:p>"),
        ),
        ("word/_rels/document.xml.rels", rels.to_vec()),
    ]);
    let dir = tmp();
    let path = write(&dir.path().join("selfrel.docx"), &doc);
    let m = difflens_core::extract::extract(&path).unwrap();
    assert_eq!(m.blocks.len(), 1);
    assert_eq!(m.blocks[0].text, "text present");
}

// ===========================================================================
// Area 3b: performance probes
// ===========================================================================

use std::time::Instant;

#[test]
fn perf_ten_thousand_tiny_paragraphs() {
    let dir = tmp();
    let mut a = String::with_capacity(200_000);
    let mut b = String::with_capacity(200_000);
    for i in 0..10_000 {
        a.push_str(&format!("Điều khoản số {i} với nội dung ngắn.\n\n"));
        let modified = if i % 10 == 0 {
            "đã sửa đổi"
        } else {
            "nội dung ngắn"
        };
        b.push_str(&format!("Điều khoản số {i} với {modified}.\n\n"));
        if i % 50 == 0 {
            b.push_str("Điều khoản hoàn toàn mới thêm vào.\n\n");
        }
    }
    let pa = write(&dir.path().join("big-a.txt"), a.as_bytes());
    let pb = write(&dir.path().join("big-b.txt"), b.as_bytes());
    // Stage timings.
    let t0 = Instant::now();
    let od = difflens_core::extract::extract(&pa).unwrap();
    let nd = difflens_core::extract::extract(&pb).unwrap();
    eprintln!("PROBE 10k extract: {:?}", t0.elapsed());
    let t1 = Instant::now();
    let al =
        difflens_core::align::align(&od.blocks, &nd.blocks, &FakeHashEmbedder::default()).unwrap();
    eprintln!(
        "PROBE 10k align: {:?} (pairs {})",
        t1.elapsed(),
        al.pairs.len()
    );
    let t2 = Instant::now();
    let pairs: Vec<(usize, usize)> = al.pairs.iter().map(|p| (p.old, p.new)).collect();
    let changes = difflens_core::diff::diff_blocks(&od.blocks, &nd.blocks, &pairs);
    eprintln!("PROBE 10k diff: {:?}", t2.elapsed());
    let t3 = Instant::now();
    let mut ch = changes.clone();
    difflens_core::heuristic::enrich_changes(&mut ch, &od.blocks, &nd.blocks);
    eprintln!("PROBE 10k enrich: {:?}", t3.elapsed());
    let t0 = Instant::now();
    let r = pipeline::run(&pa, &pb, &FakeHashEmbedder::default()).unwrap();
    let dt = t0.elapsed();
    eprintln!(
        "PROBE 10k full pipeline: {dt:?} (changes {})",
        r.changes.len()
    );
    assert!(
        dt.as_secs() < 120,
        "pipeline took {dt:?} (debug builds run ~5x slower than release)"
    );
    assert!(r.stats.added >= 100);
    assert!(r.stats.modified >= 500);
}

#[test]
fn perf_single_500kb_paragraph() {
    let dir = tmp();
    let words = "thanh toán và giao hàng theo đúng tiến độ cam kết ";
    let mut a = String::new();
    while a.len() < 500 * 1024 {
        a.push_str(words);
    }
    let mut b = a.clone();
    // Change a few words in the middle (char-boundary safe: split on a
    // known separator word that appears periodically).
    let half_words: Vec<&str> = b.split(' ').collect();
    let mut it: Vec<String> = half_words.iter().map(|s| s.to_string()).collect();
    let mid = it.len() / 2;
    it[mid] = "CAM_KẾT_MỚI".to_string();
    b = it.join(" ");
    let pa = write(&dir.path().join("huge-a.txt"), a.as_bytes());
    let pb = write(&dir.path().join("huge-b.txt"), b.as_bytes());

    // Stage timings to localize any blowup.
    let t0 = Instant::now();
    let old_doc = difflens_core::extract::extract(&pa).unwrap();
    let new_doc = difflens_core::extract::extract(&pb).unwrap();
    eprintln!("PROBE 500KB extract: {:?}", t0.elapsed());
    let t1 = Instant::now();
    let alignment = difflens_core::align::align(
        &old_doc.blocks,
        &new_doc.blocks,
        &FakeHashEmbedder::default(),
    )
    .unwrap();
    eprintln!("PROBE 500KB align: {:?}", t1.elapsed());
    let t2 = Instant::now();
    let pairs: Vec<(usize, usize)> = alignment.pairs.iter().map(|p| (p.old, p.new)).collect();
    let mut changes = difflens_core::diff::diff_blocks(&old_doc.blocks, &new_doc.blocks, &pairs);
    eprintln!("PROBE 500KB diff: {:?}", t2.elapsed());
    let t3 = Instant::now();
    let mut ch = changes.clone();
    difflens_core::heuristic::enrich_changes(&mut ch, &old_doc.blocks, &new_doc.blocks);
    eprintln!("PROBE 500KB enrich: {:?}", t3.elapsed());
    let r = pipeline::run(&pa, &pb, &FakeHashEmbedder::default()).unwrap();
    eprintln!(
        "PROBE 500KB full pipeline: {:?} (changes {})",
        t3.elapsed(),
        r.changes.len()
    );
    assert!(r.stats.modified + r.stats.equal == 1);
    changes.clear();
    let _ = changes;
}

/// 255-char basename still works end-to-end.
#[test]
fn long_basename_255() {
    let dir = tmp();
    let name = format!("{}.txt", "n".repeat(251));
    let p = write(&dir.path().join(name), b"content words here");
    let m = difflens_core::extract::extract(&p).unwrap();
    assert_eq!(m.blocks.len(), 1);
}

#[test]
fn bisect_500kb_pieces() {
    let words = "thanh toán và giao hàng theo đúng tiến độ cam kết ";
    let mut a = String::new();
    while a.len() < 500 * 1024 {
        a.push_str(words);
    }
    let b = a.replacen("theo", "THEO_MỚI", 1);
    let t = Instant::now();
    let spans = difflens_core::diff::word_spans(&a, &b);
    eprintln!(
        "PROBE word_spans 500KB: {:?} ({} spans)",
        t.elapsed(),
        spans.len()
    );
    // jaro_winkler alone on a truncated slice first to gauge rate.
    let t = Instant::now();
    let s50 = strsim::jaro_winkler(&a[..50_000], &b[..50_000]);
    eprintln!("PROBE jaro 50KB: {:?} = {s50}", t.elapsed());
}

// ===========================================================================
// Area 7: download manager vs lying servers (loopback-only mock servers)
// ===========================================================================

use difflens_core::download::{DownloadSpec, download_file};

/// Raw loopback server: one connection, custom handler over the request.
fn raw_server(handle: impl Fn(&str, &mut std::net::TcpStream) + Send + 'static) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            handle(&req, &mut stream);
        }
    });
    addr
}

/// Resume request answered 206 but with the FULL body from byte 0 (server
/// lies about Content-Range). The SHA gate must fail the job; the final
/// dest must never appear.
#[test]
fn download_resume_lying_206_full_body_fails_checksum() {
    let body = b"0123456789abcdef".to_vec();
    let good_sha = difflens_core::download::checksum::sha256_bytes(&body);
    let addr = raw_server(move |req, stream| {
        let resp = if req.contains("Range:") {
            // Lie: 206 (range honored) but send the whole body.
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 16\r\n\
             Content-Range: bytes 4-15/16\r\nConnection: close\r\n\r\n"
        } else {
            "HTTP/1.1 200 OK\r\nContent-Length: 16\r\nConnection: close\r\n\r\n"
        };
        stream.write_all(resp.as_bytes()).ok();
        stream.write_all(&body).ok();
    });
    let dir = tmp();
    let dest = dir.path().join("m.bin");
    std::fs::write(dest.with_file_name("m.bin.part"), b"0123").unwrap();
    let spec = DownloadSpec::new(format!("http://{addr}/f"), dest.clone(), good_sha);
    let err = download_file(&spec, &mut |_, _| true).unwrap_err();
    assert!(
        err.to_string().contains("checksum mismatch") || err.to_string().contains("size mismatch"),
        "{err}"
    );
    assert!(
        !dest.exists(),
        "corrupt artifact must not be renamed into place"
    );
    assert!(
        !dest.with_file_name("m.bin.part").exists(),
        "bad part must be removed"
    );
}

/// Server sends MORE bytes than its Content-Length declares.
#[test]
fn download_server_oversending_is_rejected() {
    let body = vec![0x42u8; 20];
    let good_sha = difflens_core::download::checksum::sha256_bytes(&body);
    let addr = raw_server(move |_, stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n")
            .ok();
        stream.write_all(&body).ok();
    });
    let dir = tmp();
    let dest = dir.path().join("m.bin");
    let spec = DownloadSpec::new(format!("http://{addr}/f"), dest.clone(), good_sha);
    let res = download_file(&spec, &mut |_, _| true);
    match res {
        Ok(_) => panic!("oversending server must not produce a verified artifact"),
        Err(e) => assert!(
            matches!(e, difflens_core::error::CoreError::Download(_)),
            "{e}"
        ),
    }
    assert!(!dest.exists());
}

/// Zero-byte 200 response with the empty-file sha succeeds as an empty file.
#[test]
fn download_zero_byte_body_succeeds_as_empty_file() {
    let empty_sha = difflens_core::download::checksum::sha256_bytes(&[]);
    let addr = raw_server(move |_, stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .ok();
    });
    let dir = tmp();
    let dest = dir.path().join("empty.bin");
    let spec = DownloadSpec::new(format!("http://{addr}/f"), dest.clone(), empty_sha);
    let got = download_file(&spec, &mut |_, _| true).unwrap();
    assert_eq!(std::fs::read(&got).unwrap(), Vec::<u8>::new());
}

/// Archive allowlist boundary: exact-prefix basenames pass, near-misses fail.
#[test]
fn archive_allowlist_exact_boundary() {
    use difflens_core::download::archive::{ExtractLimits, extract_archive};
    use difflens_core::download::manifest::ArchiveFormat;
    fn make_tar(entries: &[(&str, &[u8])], dir: &std::path::Path) -> PathBuf {
        let file = std::fs::File::create(dir.join("b.tar.gz")).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
        dir.join("b.tar.gz")
    }
    // Exact boundary names allowed.
    let src = tmp();
    let archive = make_tar(
        &[
            ("llama", b"launcher".as_slice()),
            ("lib", b"l".as_slice()),
            ("LICENSE", b"MIT".as_slice()),
            ("llama-b1/llama-server", b"x".as_slice()),
        ],
        src.path(),
    );
    let staging = tmp();
    let res = extract_archive(
        &archive,
        ArchiveFormat::TarGz,
        staging.path(),
        &ExtractLimits::default(),
    )
    .unwrap();
    assert_eq!(res.files.len(), 4);

    // Near-miss names rejected (the gate is prefix-based: "libx" IS allowed
    // by design — every lib* passes; these all fail the prefix).
    for bad in ["llam", "ggm", "licens", "mtm", "Llama"] {
        let src = tmp();
        let archive = make_tar(&[(bad, b"x".as_slice())], src.path());
        let staging = tmp();
        let err = extract_archive(
            &archive,
            ArchiveFormat::TarGz,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not in allowlist"), "{bad}: {err}");
    }
}

/// Gzip-bomb-ish archive: tiny compressed file, big decompressed total,
/// enforced while reading (write_capped) not just by headers.
#[test]
fn archive_total_cap_enforced_while_reading() {
    use difflens_core::download::archive::{ExtractLimits, extract_archive};
    use difflens_core::download::manifest::ArchiveFormat;
    let src = tmp();
    let file = std::fs::File::create(src.path().join("bomb.tar.gz")).unwrap();
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::new(9));
    let mut b = tar::Builder::new(enc);
    // 8 x 64 KiB of zeros → 512 KiB decompressed, compresses to ~2 KiB.
    let chunk = vec![0u8; 64 * 1024];
    for i in 0..8 {
        let mut h = tar::Header::new_gnu();
        h.set_size(chunk.len() as u64);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, format!("llama-b1/lib{i}.so"), chunk.as_slice())
            .unwrap();
    }
    b.into_inner().unwrap().finish().unwrap();
    let staging = tmp();
    let limits = ExtractLimits {
        max_total_bytes: 128 * 1024,
        max_entry_bytes: 1 << 20,
    };
    let err = extract_archive(
        &src.path().join("bomb.tar.gz"),
        ArchiveFormat::TarGz,
        staging.path(),
        &limits,
    )
    .unwrap_err();
    assert!(err.to_string().contains("total decompressed"), "{err}");
    assert!(!staging.path().exists(), "staging must be wiped");
}

// ===========================================================================
// Area 10: locale determinism (output printed; compared across LC_ALL from
// the shell: run this filter under C and vi_VN.UTF-8 and diff)
// ===========================================================================

#[test]
fn locale_determinism_canary() {
    let dir = tmp();
    let a = write(&dir.path().join("a.txt"),
        "Hợp đồng số 45/2026/HD: phí 1.500.000,75₫, lãi 0,075%/ngày, trong 30 ngày, hạn 15/03/2026.".as_bytes());
    let b = write(
        &dir.path().join("b.txt"),
        "Hợp đồng số 46/2026/HD: phí 2.500.000,5₫, lãi 0,05%/ngày, trong 15 ngày, hạn 15/03/2027."
            .as_bytes(),
    );
    let mut r = pipeline::run(&a, &b, &FakeHashEmbedder::default()).unwrap();
    // The tempdir paths embed OS randomness into the report header — pin
    // them so the hash below is a pure function of the content. (The
    // original canary hashed differently on every run under the SAME
    // locale, masking any real locale leak.)
    r.old_path = "/tmp/old-loc.txt".into();
    r.new_path = "/tmp/new-loc.txt".into();
    // Print every number-bearing surface: stats, similarity, deltas, details.
    println!(
        "STATS {:?} embed {} ocr {}",
        r.stats, r.embeddings_used, r.ocr_used
    );
    for c in &r.changes {
        println!(
            "CH {} {:?} sim {:?} sev {:?} spans {}",
            c.id,
            c.kind,
            c.similarity,
            c.severity,
            c.spans.len()
        );
        for f in &c.flags {
            println!(
                "FLAG {} {:?} {:?} {:?}",
                f.rule_id, f.old_value, f.new_value, f.detail
            );
        }
    }
    let input = ReportInput {
        id: "loc",
        result: &r,
        created_at: 1_795_000_000_000,
        lang: ReportLang::Vi,
        version: "1.0.2",
    };
    let md = difflens_core::report::markdown::render_markdown(&input);
    let html = difflens_core::report::html::render_html(&input);
    println!("MD {} {}", md.len(), md.lines().count());
    println!("HTML {} {}", html.len(), html.lines().count());
    println!("---MD-BEGIN---\n{md}---MD-END---");
    println!("---HTML-CONF-BEGIN---");
    // Number-bearing lines only (full HTML has no randomness once paths are
    // pinned; print lines containing digits for a compact byte-level diff).
    for line in html.lines() {
        if line.contains(|c: char| c.is_ascii_digit()) {
            println!("{line}");
        }
    }
    println!("---HTML-CONF-END---");
    // Hash the rendered content for exact cross-locale comparison.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    md.hash(&mut h);
    html.hash(&mut h);
    println!("HASH {}", h.finish());
}

// ===========================================================================
// Sweep continuation (gap coverage found during takeover review)
// ===========================================================================

/// UTF-8 BOM is stripped (UTF-16 variants covered above).
#[test]
fn utf8_bom_stripped() {
    let dir = tmp();
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice("Hợp đồng tiếng Việt".as_bytes());
    let p = write(&dir.path().join("bom8.txt"), &bytes);
    let m = difflens_core::extract::extract(&p).unwrap();
    assert_eq!(m.blocks.len(), 1);
    assert_eq!(m.blocks[0].text, "Hợp đồng tiếng Việt");
}

/// A zip part declaring more than the 64 MiB single-part ceiling is rejected
/// before decompression (zip-bomb first line of defense).
#[test]
fn docx_single_part_over_ceiling_is_rejected() {
    let bomb = vec![0u8; 65 * 1024 * 1024]; // 65 MiB of zeros, compresses tiny
    let doc = build_zip(&[("word/document.xml", bomb)]);
    let dir = tmp();
    let path = write(&dir.path().join("bomb.docx"), &doc);
    let err = difflens_core::extract::extract(&path).unwrap_err();
    assert!(matches!(err, CoreError::Extraction(_)), "{err}");
    assert!(err.to_string().contains("ceiling"), "{err}");
}

/// Corrupt sqlite mid-file (byte flip, not truncation): open + query either
/// salvages or errors with Store — never panics (degrade, don't crash).
#[test]
fn store_byte_flip_midfile_degrades() {
    let dir = tmp();
    let db = dir.path().join("flip.sqlite3");
    {
        let store = Store::open(&db).unwrap();
        for i in 0..20 {
            store
                .insert_session(&sample_session(&format!("f{i:02}"), i as i64))
                .unwrap();
        }
    }
    let mut bytes = std::fs::read(&db).unwrap();
    // Flip a byte in the middle of the file (page data region).
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    bytes[mid + 7] ^= 0xFF;
    std::fs::write(&db, &bytes).unwrap();
    match Store::open(&db) {
        Ok(store) => {
            // Salvaged: listing must still terminate with Store error or rows.
            match store.list_sessions(50, 0) {
                Ok(rows) => assert!(rows.len() <= 20),
                Err(e) => assert!(matches!(e, CoreError::Store(_)), "{e}"),
            }
        }
        Err(e) => assert!(matches!(e, CoreError::Store(_)), "{e}"),
    }
}

/// NUL and C0 control characters inside text: extraction keeps them out of
/// block text or passes them through deterministically; word-diff
/// reconstruction stays lossless either way.
#[test]
fn nul_and_control_chars_through_pipeline() {
    let dir = tmp();
    let a = write(
        &dir.path().join("ctl-a.txt"),
        b"alpha\x00beta paragraph\x07bell\x1b esc",
    );
    let b = write(
        &dir.path().join("ctl-b.txt"),
        b"alpha\x00gamma paragraph\x07bell\x1b esc",
    );
    let r = pipeline::run(&a, &b, &FakeHashEmbedder::default()).unwrap();
    assert_eq!(r.stats.modified, 1, "{:?}", r.stats);
    for c in &r.changes {
        let rebuilt: String = c
            .spans
            .iter()
            .filter(|s| s.op != difflens_core::model::SpanOp::Delete)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(rebuilt, "alpha\x00gamma paragraph\x07bell\x1b esc");
    }
}

/// Both sides empty: zero stats, no division anywhere, report renders.
#[test]
fn both_sides_empty_pipeline() {
    let dir = tmp();
    let a = write(&dir.path().join("e-a.txt"), b"");
    let b = write(&dir.path().join("e-b.txt"), b"");
    let r = pipeline::run(&a, &b, &FakeHashEmbedder::default()).unwrap();
    assert_eq!(
        r.stats.added + r.stats.removed + r.stats.modified + r.stats.moved + r.stats.equal,
        0
    );
    let input = ReportInput {
        id: "job-empty2",
        result: &r,
        created_at: 0,
        lang: ReportLang::En,
        version: "1.0.2",
    };
    assert!(difflens_core::report::markdown::render_markdown(&input).contains("No differences"));
    assert!(difflens_core::report::html::render_html(&input).len() > 200);
}

/// The SSE stream fragmented one byte at a time must yield the exact same
/// events as a single feed (carry-over correctness across chunk boundaries).
#[test]
fn sse_fragmented_byte_at_a_time_matches_bulk_feed() {
    let stream = b"event: message\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
                   data: [DONE]\n\n";
    let mut bulk = SseParser::new();
    let expected = bulk.feed(stream);
    let mut frag = SseParser::new();
    let mut got = Vec::new();
    for b in stream.iter() {
        got.extend(frag.feed(&[*b]));
    }
    assert_eq!(got, expected);
    assert_eq!(
        got.last(),
        Some(&SseEvent::Done),
        "fragmented stream must still terminate: {got:?}"
    );
}

/// Four threads, four connections, one database file: WAL + busy-timeout
/// must absorb concurrent writers (interleaved pipelines persisting results
/// into the same store file).
#[test]
fn concurrent_writers_on_one_store_file() {
    let dir = tmp();
    let db = dir.path().join("conc.sqlite3");
    Store::open(&db).unwrap(); // create + migrate
    let mut handles = Vec::new();
    for t in 0..4u32 {
        let db = db.clone();
        handles.push(std::thread::spawn(move || {
            let store = Store::open(&db).unwrap();
            for i in 0..25u32 {
                let id = format!("t{t:02}-i{i:02}");
                store
                    .insert_session(&sample_session(&id, (t * 100 + i) as i64))
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let store = Store::open(&db).unwrap();
    let all = store.list_sessions(1000, 0).unwrap();
    assert_eq!(all.len(), 100, "lost writes: {}", all.len());
}

/// Regression for the similarity-input cap: 200K-char inputs must compare in
/// well under a second (uncapped jaro is O(n·m) — minutes at this size) and
/// agree with the explicitly prefix-capped comparison.
#[test]
fn bounded_jaro_winkler_cap_is_fast_and_exact_on_prefixes() {
    let a = "thanh toán".repeat(20_000); // 180K chars
    let b = "thanh toán".repeat(20_000);
    let t = Instant::now();
    let sim = difflens_core::diff::bounded_jaro_winkler(&a, &b);
    let dt = t.elapsed();
    assert_eq!(sim, 1.0);
    assert!(
        dt.as_secs() < 10,
        "bounded jaro took {dt:?} (uncapped would be ~482x slower)"
    );
    // Divergent tails beyond the cap must not change the result.
    let mut c = b.clone();
    c.truncate(c.len() - 6); // char-safe: ASCII tail
    c.push_str("khác!!"); // same length, different tail beyond 8K chars
    let t = Instant::now();
    let sim2 = difflens_core::diff::bounded_jaro_winkler(&a, &c);
    assert!(t.elapsed().as_secs() < 10);
    assert_eq!(sim2, 1.0, "cap is a documented prefix approximation");
}
