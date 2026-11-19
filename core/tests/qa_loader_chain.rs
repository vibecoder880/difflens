//! QA test 8 — PDFium loader chain (packaging regression coverage for the
//! v1.0.1 Windows bug: the installed app shipped without PDFium).
//!
//! Each resolution path needs its own process (pdfium-render keeps one
//! bindings instance per thread, so in-process env switching cannot prove
//! which copy bound). The orchestrator test re-invokes this same test
//! binary with a `DIFFLENS_QA_SCENARIO` variable per child; without that
//! variable (a plain `cargo test` run) the scenario tests are no-ops.
//!
//! Scenarios:
//! - `bundled`: `DIFFLENS_RESOURCE_DIR` set to a fake app-resources dir
//!   containing `pdfium/libpdfium.so` (simulates the packaged app).
//! - `staged`: no env vars at all; the fake dir is deleted first, so the
//!   loader must fall through to the repo-staged copy.
//! - `explicit_missing`: `DIFFLENS_PDFIUM_PATH` set to a nonexistent file
//!   must FAIL — the loader never silently falls back from an explicit
//!   override.
//! - `nothing`: no override, no resource dir, no staged copy (faked via a
//!   moved CWD and a poisoned manifest-dir fallback is impossible in a
//!   test binary — covered instead by `explicit_missing`).

use std::path::PathBuf;
use std::process::Command;

const FAKE_RESOURCES: &str = "/tmp/dl-qa/fake-resources";

fn scenario() -> Option<String> {
    std::env::var("DIFFLENS_QA_SCENARIO")
        .ok()
        .filter(|s| !s.is_empty())
}

fn contract_pdf() -> String {
    let p = PathBuf::from("/tmp/dl-qa/contract-v1.pdf");
    assert!(
        p.is_file(),
        "missing {} — run: python3 -I /tmp/dl-qa/gen_docs.py",
        p.display()
    );
    p.to_str().unwrap().to_string()
}

/// Real extraction through the public pipeline entry point.
fn extract_pdf_works(note: &str) {
    let old = contract_pdf();
    let new = PathBuf::from("/tmp/dl-qa/contract-v2.pdf")
        .to_str()
        .unwrap()
        .to_string();
    let result = difflens_core::pipeline::run(
        &old,
        &new,
        &difflens_core::embeddings::FakeHashEmbedder::default(),
    )
    .unwrap_or_else(|e| panic!("{note}: extraction failed: {e}"));
    assert!(
        result.changes.len() >= 10,
        "{note}: implausible change count {}",
        result.changes.len()
    );
    assert!(
        result
            .changes
            .iter()
            .any(|c| c.spans.iter().any(|s| s.text.contains("0,075%"))),
        "{note}: pdfium text extraction lost the payment clause"
    );
}

#[test]
fn scenario_bundled_resource_dir() {
    if scenario().as_deref() != Some("bundled") {
        return; // no-op under a plain cargo test run
    }
    // SAFETY: dedicated child process for this scenario.
    unsafe {
        std::env::remove_var("DIFFLENS_PDFIUM_PATH");
        std::env::set_var("DIFFLENS_RESOURCE_DIR", FAKE_RESOURCES);
    }
    assert!(
        PathBuf::from(FAKE_RESOURCES).join("pdfium").join("libpdfium.so").is_file(),
        "fake resources dir was not staged by the orchestrator"
    );
    extract_pdf_works("bundled-resource path");
}

#[test]
fn scenario_staged_fallback() {
    if scenario().as_deref() != Some("staged") {
        return;
    }
    unsafe {
        std::env::remove_var("DIFFLENS_PDFIUM_PATH");
        std::env::remove_var("DIFFLENS_RESOURCE_DIR");
    }
    assert!(
        !PathBuf::from(FAKE_RESOURCES).exists(),
        "orchestrator must delete the fake resources dir before this scenario"
    );
    extract_pdf_works("staged-copy fallback");
}

#[test]
fn scenario_explicit_path_missing_fails() {
    if scenario().as_deref() != Some("explicit_missing") {
        return;
    }
    unsafe {
        std::env::set_var("DIFFLENS_PDFIUM_PATH", "/tmp/dl-qa/no-such-dir/libpdfium.so");
    }
    let old = contract_pdf();
    let err = difflens_core::pipeline::run(
        &old,
        &old,
        &difflens_core::embeddings::FakeHashEmbedder::default(),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("DIFFLENS_PDFIUM_PATH"),
        "error must name the failing override, got: {msg}"
    );
    assert!(
        !msg.contains("PDFium bound from system"),
        "explicit override must never silently fall back"
    );
}

/// Orchestrator: runs every scenario in a fresh child process with the
/// right environment, asserting the full chain end-to-end.
#[test]
fn qa_loader_chain_orchestrator() {
    let exe = std::env::current_exe().expect("test exe path");
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let staged = repo
        .join("resources")
        .join("binaries")
        .join("pdfium")
        .join("lib")
        .join("libpdfium.so");
    assert!(staged.is_file(), "staged pdfium missing at {}", staged.display());

    // Stage the fake packaged-app resources dir.
    let fake = PathBuf::from(FAKE_RESOURCES);
    let _ = std::fs::remove_dir_all(&fake);
    std::fs::create_dir_all(fake.join("pdfium")).unwrap();
    std::fs::copy(&staged, fake.join("pdfium").join("libpdfium.so")).unwrap();

    let run_scenario = |name: &str, envs: &[(&str, Option<String>)]| {
        let mut cmd = Command::new(&exe);
        cmd.arg(name)
            .arg("--exact")
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env_remove("DIFFLENS_QA_SCENARIO")
            .env("DIFFLENS_QA_SCENARIO", name);
        for (k, v) in envs {
            match v {
                Some(val) => {
                    cmd.env(k, val);
                }
                None => {
                    cmd.env_remove(k);
                }
            }
        }
        let out = cmd.output().expect("spawn scenario child");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    // 1. Bundled-resource path (the NEW packaged-app resolution).
    let (ok, out, err) = run_scenario(
        "scenario_bundled_resource_dir",
        &[("DIFFLENS_PDFIUM_PATH", None), ("DIFFLENS_RESOURCE_DIR", Some(FAKE_RESOURCES.into()))],
    );
    assert!(ok, "bundled-resource scenario failed\nstdout:\n{out}\nstderr:\n{err}");

    // 2. Delete the fake dir → the loader must fall through to the staged copy.
    std::fs::remove_dir_all(&fake).unwrap();
    let (ok, out, err) = run_scenario(
        "scenario_staged_fallback",
        &[("DIFFLENS_PDFIUM_PATH", None), ("DIFFLENS_RESOURCE_DIR", None)],
    );
    assert!(ok, "staged-fallback scenario failed\nstdout:\n{out}\nstderr:\n{err}");

    // 3. Explicit-but-missing override fails loudly (no silent fallback).
    let (ok, out, err) = run_scenario(
        "scenario_explicit_path_missing_fails",
        &[("DIFFLENS_PDFIUM_PATH", Some("/tmp/dl-qa/no-such-dir/libpdfium.so".into()))],
    );
    assert!(ok, "explicit-missing scenario failed\nstdout:\n{out}\nstderr:\n{err}");
}
