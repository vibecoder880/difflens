//! Standalone HTML report: single file, inline CSS, zero JavaScript —
//! opens from disk in any browser.
//!
//! **Security invariant (phase-08 SEC1):** every document-derived string
//! (file names/paths, block text, inline spans, flag values, rule details,
//! page references) passes through [`html_escape`] before entering the
//! markup. Documents are attacker-controlled; an unescaped export would be
//! a stored-XSS payload opened in a browser. The hostile-content golden
//! test (`core/tests/report_tests.rs`) enforces this.

use std::fmt::Write as _;

use crate::model::{Change, ChangeKind, DiffSpan, SpanOp};
use crate::report::{KIND_ORDER, Labels, ReportInput, flag_label, format_delta, format_utc_ms};

/// Escape a string for safe inclusion in HTML text content and (defensively)
/// attribute values. Applied to **all** document-derived strings — there is
/// no code path where raw document text reaches the output.
///
/// Beyond markup characters, unicode bidirectional control characters are
/// replaced with numeric references so hostile text cannot visually
/// re-order surrounding report markup (RTL-spoofing).
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => {
                let _ = write!(out, "&#x{:04X};", c as u32);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Render the full standalone HTML report.
pub fn render_html(input: &ReportInput) -> String {
    let labels = input.labels();
    let mut out = String::with_capacity(16 * 1024);

    let _ = writeln!(
        out,
        "<!doctype html>\n<html lang=\"{}\">",
        labels.lang.html_lang()
    );
    out.push_str(
        "<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" \
         content=\"width=device-width, initial-scale=1\">\n<title>",
    );
    let _ = writeln!(
        out,
        "{} — DiffLens</title>",
        html_escape(&format!(
            "{} · {}",
            base_name(&input.result.old_path),
            base_name(&input.result.new_path)
        ))
    );
    push_style(&mut out);
    out.push_str("</head>\n<body>\n");
    push_header(&mut out, input, labels);
    push_summary(&mut out, input, labels);
    push_changes(&mut out, input, labels);
    push_audit(&mut out, input, labels);
    push_footer(&mut out, input, labels);
    out.push_str("</body>\n</html>\n");
    out
}

/// Last path component (both separators), e.g. `/tmp/a b/contract.docx` →
/// `contract.docx`.
pub(crate) fn base_name(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

fn push_header(out: &mut String, input: &ReportInput, labels: &Labels) {
    let r = &input.result;
    let _ = write!(out, "<header>\n<h1>{}</h1>\n", html_escape(labels.title));
    out.push_str("<table class=\"meta\">\n<tbody>\n");
    meta_row(out, labels.old_file, &r.old_path);
    meta_row(out, labels.new_file, &r.new_path);
    meta_row(
        out,
        labels.format,
        &format!("{:?}", r.format).to_uppercase(),
    );
    meta_row(out, labels.generated, &format_utc_ms(input.created_at));
    meta_row(out, labels.session_id, input.id);
    out.push_str("</tbody>\n</table>\n");
    // Alignment / OCR notes as a chips line (no document-derived content).
    let alignment = if r.embeddings_used {
        labels.alignment_on
    } else {
        labels.alignment_off
    };
    let _ = write!(
        out,
        "<p class=\"note chips\">{}</p>",
        html_escape(alignment)
    );
    if r.ocr_used {
        let _ = write!(
            out,
            "<p class=\"note chips\">{}</p>",
            html_escape(labels.ocr_note)
        );
    }
    out.push_str("</header>\n");
}

fn meta_row(out: &mut String, label: &str, value: &str) {
    let _ = writeln!(
        out,
        "<tr><th scope=\"row\">{}</th><td>{}</td></tr>",
        html_escape(label),
        html_escape(value)
    );
}

fn push_summary(out: &mut String, input: &ReportInput, labels: &Labels) {
    let s = &input.result.stats;
    let _ = write!(
        out,
        "<section id=\"summary\">\n<h2>{}</h2>\n",
        html_escape(labels.summary)
    );
    out.push_str("<table class=\"stats\">\n<thead><tr><th>");
    let _ = write!(
        out,
        "{}</th><th>{}</th></tr></thead>\n<tbody>\n",
        html_escape(labels.summary),
        html_escape(labels.change)
    );
    stat_row(out, "added", labels.st_added, s.added);
    stat_row(out, "removed", labels.st_removed, s.removed);
    stat_row(out, "modified", labels.st_modified, s.modified);
    stat_row(out, "moved", labels.st_moved, s.moved);
    stat_row(out, "equal", labels.st_equal, s.equal);
    out.push_str("</tbody>\n</table>\n</section>\n");
}

fn stat_row(out: &mut String, class: &str, label: &str, count: usize) {
    let _ = writeln!(
        out,
        "<tr class=\"stat-{class}\"><td>{}</td><td class=\"num\">{}</td></tr>",
        html_escape(label),
        count
    );
}

fn push_changes(out: &mut String, input: &ReportInput, labels: &Labels) {
    let _ = write!(
        out,
        "<section id=\"changes\">\n<h2>{}</h2>\n",
        html_escape(labels.changes)
    );
    if input.result.changes.is_empty() {
        let _ = writeln!(
            out,
            "<p>{}</p>\n</section>\n",
            html_escape(labels.no_changes)
        );
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
        let _ = writeln!(
            out,
            "<h3 class=\"kind kind-{}\">{} ({})</h3>",
            kind_name(kind),
            html_escape(labels.kind(kind)),
            group.len()
        );
        for change in group {
            push_change(out, change, labels);
        }
    }
    out.push_str("</section>\n");
}

fn push_change(out: &mut String, change: &Change, labels: &Labels) {
    let _ = writeln!(
        out,
        "<article class=\"change\" id=\"{}\">",
        html_escape(&change.id)
    );
    // Head line: id + severity badge (+ page + similarity meta).
    out.push_str("<div class=\"change-head\">");
    let _ = write!(
        out,
        "<span class=\"change-id\">{}</span>",
        html_escape(&change.id)
    );
    if let Some(sev) = change.severity {
        let _ = write!(
            out,
            "<span class=\"badge sev-{}\">{}</span>",
            sev_name(sev),
            html_escape(labels.severity(sev))
        );
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
        let _ = write!(
            out,
            "<span class=\"meta\">{}</span>",
            html_escape(&meta.join(" · "))
        );
    }
    out.push_str("</div>\n");

    // Flags first (they are the reason a change matters).
    push_flags(out, change, labels);

    // Inline spans (Modified/Moved).
    if !change.spans.is_empty() {
        out.push_str("<p class=\"spans\">\n");
        for span in &change.spans {
            push_span(out, span);
        }
        out.push_str("\n</p>\n");
    }
    // Full removed / added text.
    if let Some(old) = &change.old_text {
        let _ = writeln!(
            out,
            "<pre class=\"old-text\"><span class=\"sign\">−</span>{}</pre>",
            html_escape(old)
        );
    }
    if let Some(new) = &change.new_text {
        let _ = writeln!(
            out,
            "<pre class=\"new-text\"><span class=\"sign\">+</span>{}</pre>",
            html_escape(new)
        );
    }
    out.push_str("</article>\n");
}

fn push_span(out: &mut String, span: &DiffSpan) {
    let escaped = html_escape(&span.text);
    match span.op {
        SpanOp::Equal => {
            let _ = write!(out, "<span class=\"eq\">{escaped}</span>");
        }
        SpanOp::Delete => {
            let _ = write!(out, "<del class=\"del\">{escaped}</del>");
        }
        SpanOp::Insert => {
            let _ = write!(out, "<ins class=\"ins\">{escaped}</ins>");
        }
    }
}

fn push_flags(out: &mut String, change: &Change, labels: &Labels) {
    if change.flags.is_empty() {
        return;
    }
    let _ = writeln!(out, "<ul class=\"flags\">");
    for flag in &change.flags {
        let label = flag_label(labels, &flag.label_key);
        let mut parts: Vec<String> = Vec::new();
        if let Some(old) = &flag.old_value {
            parts.push(format!("{}: {}", labels.old_value, old));
        }
        if let Some(new) = &flag.new_value {
            parts.push(format!("{}: {}", labels.new_value, new));
        }
        if let Some(delta) = flag.delta {
            parts.push(format!("{}: {}", labels.delta, format_delta(delta)));
        }
        if let Some(detail) = &flag.detail {
            parts.push(format!("{}: {}", labels.detail, detail));
        }
        let _ = write!(out, "<li><strong>{}</strong>", html_escape(label));
        if !parts.is_empty() {
            let _ = write!(out, " — {}", html_escape(&parts.join(" · ")));
        }
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n");
}

fn push_audit(out: &mut String, input: &ReportInput, labels: &Labels) {
    if input.result.audit.is_empty() {
        return;
    }
    let _ = write!(
        out,
        "<section id=\"audit\">\n<h2>{}</h2>\n",
        html_escape(labels.audit)
    );
    let _ = writeln!(
        out,
        "<p class=\"note\">{}</p>",
        html_escape(labels.audit_note)
    );
    out.push_str("<table class=\"audit-table\">\n<thead><tr><th>");
    let _ = write!(
        out,
        "{}</th><th>{}</th><th>{}</th></tr></thead>\n<tbody>\n",
        html_escape(labels.change),
        html_escape(labels.summary),
        html_escape(labels.rule)
    );
    for entry in &input.result.audit {
        let _ = write!(
            out,
            "<tr><td>{}</td><td><span class=\"badge sev-{}\">{}</span></td><td>",
            html_escape(&entry.change_id),
            sev_name(entry.severity),
            html_escape(labels.severity(entry.severity))
        );
        if entry.rules.is_empty() {
            let _ = write!(out, "—");
        } else {
            out.push_str("<ul class=\"rule-list\">\n");
            for flag in &entry.rules {
                let mut bits: Vec<String> = vec![flag_label(labels, &flag.label_key).to_string()];
                if let Some(old) = &flag.old_value {
                    bits.push(format!("{}: {}", labels.old_value, old));
                }
                if let Some(new) = &flag.new_value {
                    bits.push(format!("{}: {}", labels.new_value, new));
                }
                if let Some(delta) = flag.delta {
                    bits.push(format!("{}: {}", labels.delta, format_delta(delta)));
                }
                if let Some(detail) = &flag.detail {
                    bits.push(format!("{}: {}", labels.detail, detail));
                }
                let _ = writeln!(out, "<li>{}</li>", html_escape(&bits.join(" · ")));
            }
            out.push_str("</ul>\n");
        }
        out.push_str("</td></tr>\n");
    }
    out.push_str("</tbody>\n</table>\n</section>\n");
}

fn push_footer(out: &mut String, input: &ReportInput, labels: &Labels) {
    let _ = write!(
        out,
        "<footer>\n<hr>\n<p>{}</p>\n<p class=\"version\">DiffLens {} · {}</p>\n</footer>\n",
        html_escape(labels.footer),
        html_escape(input.version),
        html_escape(&format_utc_ms(input.created_at)),
    );
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Modified => "modified",
        ChangeKind::Moved => "moved",
        ChangeKind::Equal => "equal",
    }
}

fn sev_name(sev: crate::model::Severity) -> &'static str {
    use crate::model::Severity as S;
    match sev {
        S::Info => "info",
        S::Low => "low",
        S::Medium => "medium",
        S::High => "high",
        S::Critical => "critical",
    }
}

/// Inline CSS — the report must render standalone (no external assets).
fn push_style(out: &mut String) {
    out.push_str(
        "<style>\n\
         :root { color-scheme: light; }\n\
         body { font-family: system-ui, -apple-system, \"Segoe UI\", sans-serif; margin: 0;\n\
         color: #1a1d21; background: #f6f7f8; line-height: 1.5; }\n\
         header, section, footer, article, h3 { max-width: 880px; margin-left: auto; margin-right: auto; }\n\
         header { padding: 24px 16px 8px; }\n\
         h1 { font-size: 1.35rem; margin: 0 0 12px; }\n\
         h2 { font-size: 1.1rem; margin: 24px 16px 8px; border-bottom: 1px solid #d8dce0; padding-bottom: 4px; }\n\
         h3 { font-size: 0.95rem; margin: 16px 16px 6px; }\n\
         table { border-collapse: collapse; background: #fff; border: 1px solid #d8dce0;\n\
         border-radius: 6px; width: 100%; }\n\
         th, td { text-align: left; padding: 6px 10px; vertical-align: top; font-size: 0.85rem;\n\
         border-top: 1px solid #e6e9ec; word-break: break-word; }\n\
         thead th { background: #eef1f3; font-weight: 600; }\n\
         tbody tr:first-child th, tbody tr:first-child td { border-top: 0; }\n\
         td.num { text-align: right; font-variant-numeric: tabular-nums; }\n\
         .stat-added td:first-child { color: #0a6c3c; }\n\
         .stat-removed td:first-child { color: #b3261e; }\n\
         .stat-modified td:first-child { color: #8a5a00; }\n\
         .change { background: #fff; border: 1px solid #d8dce0; border-radius: 6px;\n\
         padding: 10px 12px; margin: 8px 16px; }\n\
         .change-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }\n\
         .change-id { font-family: ui-monospace, monospace; font-size: 0.8rem; color: #5b636b; }\n\
         .meta { font-size: 0.75rem; color: #5b636b; }\n\
         .badge { display: inline-block; border-radius: 4px; padding: 1px 8px;\n\
         font-size: 0.72rem; font-weight: 600; }\n\
         .sev-info { background: #e8eefc; color: #2a4d8f; }\n\
         .sev-low { background: #e6f2ea; color: #0a6c3c; }\n\
         .sev-medium { background: #fdf2d7; color: #8a5a00; }\n\
         .sev-high { background: #fbe4d5; color: #9a3b12; }\n\
         .sev-critical { background: #f8dcdc; color: #b3261e; }\n\
         .spans { margin: 8px 0 0; }\n\
         .eq { color: #444; }\n\
         del.del { color: #b3261e; background: #fdecec; text-decoration: line-through; }\n\
         ins.ins { color: #0a6c3c; background: #e7f5ec; text-decoration: none; }\n\
         pre.old-text, pre.new-text { margin: 6px 0 0; padding: 8px 10px; border-radius: 4px;\n\
         white-space: pre-wrap; word-break: break-word; font-size: 0.8rem;\n\
         font-family: ui-monospace, monospace; }\n\
         pre.old-text { background: #fdecec; color: #7f1d1a; }\n\
         pre.new-text { background: #e7f5ec; color: #0a5231; }\n\
         .sign { font-weight: 700; margin-right: 6px; }\n\
         ul.flags { margin: 8px 0 0; padding-left: 18px; font-size: 0.82rem; }\n\
         ul.rule-list { margin: 0; padding-left: 16px; }\n\
         p.note { font-size: 0.78rem; color: #5b636b; margin: 0 16px 8px; max-width: 880px;\n\
         margin-left: auto; margin-right: auto; }\n\
         footer { padding: 16px; font-size: 0.78rem; color: #5b636b; }\n\
         footer p { max-width: 880px; margin: 4px auto; }\n\
         </style>\n",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_neutralizes_markup_and_quotes() {
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(
            html_escape("<img src=x onerror=alert(1)>"),
            "&lt;img src=x onerror=alert(1)&gt;"
        );
        assert_eq!(html_escape("\"'\""), "&quot;&#x27;&quot;");
        assert_eq!(html_escape("trẻ em"), "trẻ em"); // unicode untouched
    }

    #[test]
    fn base_names_handles_separators() {
        assert_eq!(base_name("/tmp/a/b.docx"), "b.docx");
        assert_eq!(base_name("C:\\temp\\b.docx"), "b.docx");
        assert_eq!(base_name("plain.txt"), "plain.txt");
    }
}
