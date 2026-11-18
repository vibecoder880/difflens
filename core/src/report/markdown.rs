//! Compact Markdown report: headings, metadata + stats tables, changes as
//! fenced diff blocks (`- old` / `+ new`), heuristic flags as bullets, and
//! an audit appendix.
//!
//! Markdown keeps document text **raw** inside fenced code blocks — no HTML
//! escaping — but two integrity rules apply:
//!
//! 1. Fences are chosen dynamically to be strictly longer than the longest
//!    backtick run in the content, so hostile content cannot terminate the
//!    block early (fence-integrity golden test).
//! 2. Table cells escape pipes and fold newlines so hostile content cannot
//!    break the table shape.

use std::fmt::Write as _;

use crate::model::{Change, SpanOp};
use crate::report::{KIND_ORDER, Labels, ReportInput, flag_label, format_delta, format_utc_ms};

/// Render the Markdown report.
pub fn render_markdown(input: &ReportInput) -> String {
    let labels = input.labels();
    let mut out = String::with_capacity(8 * 1024);
    let _ = writeln!(out, "# {}", labels.title);
    out.push('\n');
    push_meta(&mut out, input, labels);
    push_summary(&mut out, input, labels);
    push_changes(&mut out, input, labels);
    push_audit(&mut out, input, labels);
    let _ = writeln!(out, "---\n");
    let _ = writeln!(out, "{}", labels.footer);
    let _ = writeln!(
        out,
        "DiffLens {} · {}",
        input.version,
        format_utc_ms(input.created_at)
    );
    out
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

fn push_meta(out: &mut String, input: &ReportInput, labels: &Labels) {
    let r = &input.result;
    let _ = writeln!(out, "| | |\n|---|---|");
    let _ = writeln!(out, "| {} | {} |", labels.old_file, cell(&r.old_path));
    let _ = writeln!(out, "| {} | {} |", labels.new_file, cell(&r.new_path));
    let _ = writeln!(
        out,
        "| {} | {} |",
        labels.format,
        cell(&format!("{:?}", r.format).to_uppercase())
    );
    let _ = writeln!(
        out,
        "| {} | {} |",
        labels.generated,
        cell(&format_utc_ms(input.created_at))
    );
    let _ = writeln!(out, "| {} | {} |", labels.session_id, cell(input.id));
    let alignment = if r.embeddings_used {
        labels.alignment_on
    } else {
        labels.alignment_off
    };
    let _ = writeln!(out, "| | {} |", cell(alignment));
    if r.ocr_used {
        let _ = writeln!(out, "| | {} |", cell(labels.ocr_note));
    }
    out.push('\n');
}

fn push_summary(out: &mut String, input: &ReportInput, labels: &Labels) {
    let _ = writeln!(out, "## {}\n", labels.summary);
    let _ = writeln!(out, "| | {} |\n|---|---|", labels.change);
    let s = &input.result.stats;
    let _ = writeln!(out, "| {} | {} |", labels.st_added, s.added);
    let _ = writeln!(out, "| {} | {} |", labels.st_removed, s.removed);
    let _ = writeln!(out, "| {} | {} |", labels.st_modified, s.modified);
    let _ = writeln!(out, "| {} | {} |", labels.st_moved, s.moved);
    let _ = writeln!(out, "| {} | {} |", labels.st_equal, s.equal);
    out.push('\n');
}

fn push_changes(out: &mut String, input: &ReportInput, labels: &Labels) {
    let _ = writeln!(out, "## {}\n", labels.changes);
    if input.result.changes.is_empty() {
        let _ = writeln!(out, "{}\n", labels.no_changes);
        return;
    }
    for kind in KIND_ORDER {
        let group: Vec<&Change> = input
            .result
            .changes
            .iter()
            .filter(|c| c.kind == kind)
            .collect();
        if group.is_empty() {
            continue;
        }
        let _ = writeln!(out, "### {} ({})\n", labels.kind(kind), group.len());
        for change in group {
            push_change(out, change, labels);
        }
    }
}

fn push_change(out: &mut String, change: &Change, labels: &Labels) {
    // Head: id + severity (+ meta).
    let mut head = format!("#### `{}`", change.id);
    if let Some(sev) = change.severity {
        let _ = write!(head, " — {}", labels.severity(sev));
    }
    let mut meta: Vec<String> = Vec::new();
    if let Some(p) = change.old_page {
        meta.push(format!("{} {}", labels.page, p));
    }
    if let Some(p) = change.new_page
        && (change.old_page.is_none() || change.old_page != change.new_page)
    {
        meta.push(format!("{} {}", labels.page, p));
    }
    if let Some(sim) = change.similarity {
        meta.push(format!(
            "{}% {}",
            (sim * 100.0).round() as i64,
            labels.similarity
        ));
    }
    if !meta.is_empty() {
        let _ = write!(head, " ({})", meta.join(" · "));
    }
    let _ = writeln!(out, "{head}\n");

    // Flags as bullets.
    for flag in &change.flags {
        let mut parts: Vec<String> = Vec::new();
        if let Some(old) = &flag.old_value {
            parts.push(format!("{}: {}", labels.old_value, code(old)));
        }
        if let Some(new) = &flag.new_value {
            parts.push(format!("{}: {}", labels.new_value, code(new)));
        }
        if let Some(delta) = flag.delta {
            parts.push(format!("{}: {}", labels.delta, format_delta(delta)));
        }
        if let Some(detail) = &flag.detail {
            parts.push(format!("{}: {}", labels.detail, detail));
        }
        let _ = writeln!(
            out,
            "- {}{}",
            flag_label(labels, &flag.label_key),
            if parts.is_empty() {
                String::new()
            } else {
                format!(" — {}", parts.join(" · "))
            }
        );
    }
    if !change.flags.is_empty() {
        out.push('\n');
    }

    // Diff fences.
    match (&change.kind, change.spans.as_slice()) {
        // Modified/Moved: inline spans split into -/+ sides.
        (_, spans) if !spans.is_empty() => {
            let old: String = spans
                .iter()
                .filter(|s| s.op != SpanOp::Insert)
                .map(|s| s.text.as_str())
                .collect();
            let new: String = spans
                .iter()
                .filter(|s| s.op != SpanOp::Delete)
                .map(|s| s.text.as_str())
                .collect();
            push_diff_fence(
                out,
                if old.is_empty() { None } else { Some(&old) },
                if new.is_empty() { None } else { Some(&new) },
            );
        }
        // Removed: full old text.
        (crate::model::ChangeKind::Removed, _) => {
            push_diff_fence(out, change.old_text.as_deref(), None);
        }
        // Added: full new text.
        (crate::model::ChangeKind::Added, _) => {
            push_diff_fence(out, None, change.new_text.as_deref());
        }
        _ => {}
    }
}

/// Fenced diff block with dynamic fence length: longer than any backtick
/// run in the content so the block cannot be closed early.
fn push_diff_fence(out: &mut String, old: Option<&str>, new: Option<&str>) {
    let old_lines: Vec<&str> = old.map(|t| t.lines().collect()).unwrap_or_default();
    let new_lines: Vec<&str> = new.map(|t| t.lines().collect()).unwrap_or_default();
    let mut fence_len = 3usize;
    for text in old.into_iter().chain(new) {
        let mut run = 0usize;
        for c in text.chars() {
            if c == '`' {
                run += 1;
                fence_len = fence_len.max(run + 1);
            } else {
                run = 0;
            }
        }
    }
    let fence: String = "`".repeat(fence_len);
    let _ = writeln!(out, "{fence}");
    for line in &old_lines {
        let _ = writeln!(out, "- {line}");
    }
    for line in &new_lines {
        let _ = writeln!(out, "+ {line}");
    }
    let _ = writeln!(out, "{fence}\n");
}

fn push_audit(out: &mut String, input: &ReportInput, labels: &Labels) {
    if input.result.audit.is_empty() {
        return;
    }
    let _ = writeln!(out, "## {}\n", labels.audit);
    let _ = writeln!(out, "{}\n", labels.audit_note);
    let _ = writeln!(
        out,
        "| {} | {} | {} |\n|---|---|---|",
        labels.change, labels.summary, labels.rule
    );
    for entry in &input.result.audit {
        let sev = match entry.severity {
            crate::model::Severity::Info => labels.sev_info,
            crate::model::Severity::Low => labels.sev_low,
            crate::model::Severity::Medium => labels.sev_medium,
            crate::model::Severity::High => labels.sev_high,
            crate::model::Severity::Critical => labels.sev_critical,
        };
        if entry.rules.is_empty() {
            let _ = writeln!(out, "| `{}` | {} | — |", entry.change_id, sev);
        } else {
            let rules: Vec<String> = entry
                .rules
                .iter()
                .map(|flag| {
                    let mut bits = vec![flag_label(labels, &flag.label_key).to_string()];
                    if let Some(old) = &flag.old_value {
                        bits.push(format!("{}: {}", labels.old_value, cell(old)));
                    }
                    if let Some(new) = &flag.new_value {
                        bits.push(format!("{}: {}", labels.new_value, cell(new)));
                    }
                    if let Some(delta) = flag.delta {
                        bits.push(format!("{}: {}", labels.delta, format_delta(delta)));
                    }
                    if let Some(detail) = &flag.detail {
                        bits.push(format!("{}: {}", labels.detail, cell(detail)));
                    }
                    bits.join(" · ")
                })
                .collect();
            let _ = writeln!(
                out,
                "| `{}` | {} | {} |",
                entry.change_id,
                sev,
                rules.join("<br>")
            );
        }
    }
    out.push('\n');
}

// ---------------------------------------------------------------------------
// Cell helpers
// ---------------------------------------------------------------------------

/// Escape a string for a GFM table cell: pipes, newlines.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ").replace('\r', "")
}

/// Inline code span for raw values (backtick runs cannot occur in extracted
/// flag values that matter here, but be safe: use double-backtick fences
/// when a backtick is present).
fn code(s: &str) -> String {
    if s.contains('`') {
        format!("`` {s} ``")
    } else {
        format!("`{s}`")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_escapes_pipes_and_newlines() {
        assert_eq!(cell("a|b"), "a\\|b");
        assert_eq!(cell("line1\nline2"), "line1 line2");
    }

    #[test]
    fn fence_grows_with_backtick_runs() {
        let mut out = String::new();
        push_diff_fence(&mut out, Some("plain"), Some("has `` run"));
        assert!(out.starts_with("```\n"), "default fence: {out:?}");
        assert!(out.contains("+ has `` run"));

        let mut out2 = String::new();
        push_diff_fence(&mut out2, Some("`````"), None);
        assert!(
            out2.starts_with("``````\n"),
            "fence must exceed run: {out2:?}"
        );
    }
}
