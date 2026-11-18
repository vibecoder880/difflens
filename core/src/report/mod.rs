//! Report renderers: standalone HTML and Markdown exports of a stored
//! [`DiffResult`].
//!
//! Reports are pure functions of (result, injected timestamp, language,
//! version) — they never read the wall clock, so the same input renders
//! byte-identical output (golden-testable). Renderers do no I/O, no
//! network, no process spawning.
//!
//! Security: document-derived text (file names, block text, spans, flag
//! values, rule details) is attacker-controlled. Every such string passes
//! through [`html::html_escape`] in the HTML renderer — see the
//! hostile-content golden test. The Markdown renderer keeps content inside
//! fenced code blocks whose length is chosen dynamically to be longer than
//! any backtick run in the content.

pub mod html;
pub mod markdown;

use crate::model::{ChangeKind, DiffResult, Severity};

/// Report language (small label map mirroring the frontend i18n keys; the
/// parity test covers every `ALL_LABEL_KEYS` entry in both languages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportLang {
    En,
    Vi,
}

impl ReportLang {
    /// Parse a BCP-47-ish tag ("en", "vi", "vi-VN", …); unknown tags fall
    /// back to English.
    pub fn from_tag(tag: &str) -> ReportLang {
        let lower = tag.trim().to_ascii_lowercase();
        if lower.starts_with("vi") {
            ReportLang::Vi
        } else {
            ReportLang::En
        }
    }

    /// `<html lang>` value.
    pub fn html_lang(self) -> &'static str {
        match self {
            ReportLang::En => "en",
            ReportLang::Vi => "vi",
        }
    }
}

/// Everything a renderer needs. `created_at` is injected by the caller
/// (epoch milliseconds); renderers never read the clock.
#[derive(Debug, Clone)]
pub struct ReportInput<'a> {
    /// Job or session id the result belongs to (purely informational).
    pub id: &'a str,
    pub result: &'a DiffResult,
    pub created_at: i64,
    pub lang: ReportLang,
    /// App version rendered in the footer.
    pub version: &'a str,
}

impl ReportInput<'_> {
    /// Label table for the configured language.
    pub fn labels(&self) -> &'static Labels {
        labels(self.lang)
    }
}

/// Order used when grouping changes by kind (stable, matches the UI).
pub(crate) const KIND_ORDER: [ChangeKind; 5] = [
    ChangeKind::Modified,
    ChangeKind::Added,
    ChangeKind::Removed,
    ChangeKind::Moved,
    ChangeKind::Equal,
];

// ---------------------------------------------------------------------------
// Timestamp formatting (UTC, no external crates)
// ---------------------------------------------------------------------------

/// Signed relative delta as a percentage string, e.g. `-50%` (mirror of the
/// frontend `formatDelta`).
pub(crate) fn format_delta(delta: f64) -> String {
    let percent = (delta * 1000.0).round() / 10.0;
    let sign = if percent > 0.0 {
        "+"
    } else if percent < 0.0 {
        "−"
    } else {
        "±"
    };
    let abs = percent.abs();
    if abs.fract() == 0.0 {
        format!("{sign}{abs:.0}%")
    } else {
        format!("{sign}{abs:.1}%")
    }
}

/// Format epoch milliseconds as `YYYY-MM-DD HH:MM:SS UTC`.
///
/// Civil-from-days algorithm (Howard Hinnant) — deterministic, no libc, no
/// chrono dependency.
pub fn format_utc_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02}.{millis:03} UTC")
}

/// Days since 1970-01-01 → (year, month, day) in the proleptic Gregorian
/// calendar.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// Label tables (EN + VI; parity-tested against ALL_LABEL_KEYS)
// ---------------------------------------------------------------------------

/// Static label strings for one language. Every user-visible report string
/// lives here so EN/VI parity can be tested exhaustively.
pub struct Labels {
    pub lang: ReportLang,
    /// Report title.
    pub title: &'static str,
    pub old_file: &'static str,
    pub new_file: &'static str,
    pub generated: &'static str,
    pub format: &'static str,
    pub alignment_on: &'static str,
    pub alignment_off: &'static str,
    pub ocr_note: &'static str,
    pub session_id: &'static str,
    pub summary: &'static str,
    pub changes: &'static str,
    pub no_changes: &'static str,
    pub audit: &'static str,
    pub audit_note: &'static str,
    pub change: &'static str,
    pub rule: &'static str,
    pub old_value: &'static str,
    pub new_value: &'static str,
    pub delta: &'static str,
    pub detail: &'static str,
    pub page: &'static str,
    pub similarity: &'static str,
    /// Footer disclaimer (hard requirement).
    pub footer: &'static str,
    /// Stats / kind / severity labels.
    pub st_added: &'static str,
    pub st_removed: &'static str,
    pub st_modified: &'static str,
    pub st_moved: &'static str,
    pub st_equal: &'static str,
    pub kind_added: &'static str,
    pub kind_removed: &'static str,
    pub kind_modified: &'static str,
    pub kind_moved: &'static str,
    pub kind_equal: &'static str,
    pub sev_info: &'static str,
    pub sev_low: &'static str,
    pub sev_medium: &'static str,
    pub sev_high: &'static str,
    pub sev_critical: &'static str,
}

/// Label for a heuristic flag key (`flag.<camelCase>`); unknown keys fall
/// back to the "unrecognized signal" label rather than leaking a raw key.
pub fn flag_label(labels: &Labels, label_key: &str) -> &'static str {
    match label_key {
        "flag.moneyChanged" => match labels.lang {
            ReportLang::En => "Money amount changed",
            ReportLang::Vi => "Số tiền thay đổi",
        },
        "flag.dateChanged" => match labels.lang {
            ReportLang::En => "Date changed",
            ReportLang::Vi => "Ngày tháng thay đổi",
        },
        "flag.durationChanged" => match labels.lang {
            ReportLang::En => "Duration changed",
            ReportLang::Vi => "Thời hạn thay đổi",
        },
        "flag.entityAdded" => match labels.lang {
            ReportLang::En => "New value introduced",
            ReportLang::Vi => "Giá trị mới xuất hiện",
        },
        "flag.entityRemoved" => match labels.lang {
            ReportLang::En => "Value removed",
            ReportLang::Vi => "Giá trị bị xóa",
        },
        "flag.clauseRenamed" => match labels.lang {
            ReportLang::En => "Clause renamed",
            ReportLang::Vi => "Điều khoản bị đổi tên",
        },
        "flag.referenceChanged" => match labels.lang {
            ReportLang::En => "Reference number changed",
            ReportLang::Vi => "Số tham chiếu thay đổi",
        },
        "flag.percentChanged" => match labels.lang {
            ReportLang::En => "Percentage changed",
            ReportLang::Vi => "Tỷ lệ phần trăm thay đổi",
        },
        "flag.keywordRisk" => match labels.lang {
            ReportLang::En => "Risky keyword",
            ReportLang::Vi => "Từ khóa tiềm ẩn rủi ro",
        },
        _ => match labels.lang {
            ReportLang::En => "Unrecognized signal",
            ReportLang::Vi => "Tín hiệu không nhận dạng được",
        },
    }
}

/// All label keys the report renderers can resolve (heuristic keys +
/// fallback). Test hook for EN/VI parity.
pub fn all_report_flag_keys() -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = crate::heuristic::ALL_LABEL_KEYS.to_vec();
    keys.push("flag.unknown");
    keys.push("flag.somethingUnlisted"); // exercises the fallback arm
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// English label table.
static EN_LABELS: Labels = Labels {
    lang: ReportLang::En,
    title: "Document comparison report",
    old_file: "Old file",
    new_file: "New file",
    generated: "Generated",
    format: "Format",
    alignment_on: "Semantic alignment: on",
    alignment_off: "Semantic alignment: off (positional fallback)",
    ocr_note: "Includes pages extracted via OCR (experimental)",
    session_id: "Session",
    summary: "Summary",
    changes: "Changes",
    no_changes: "No differences found — the documents match.",
    audit: "Audit trail",
    audit_note: "Rules applied in order; the highest severity band wins.",
    change: "Change",
    rule: "Rule",
    old_value: "Old",
    new_value: "New",
    delta: "Delta",
    detail: "Detail",
    page: "Page",
    similarity: "match",
    footer: "Generated locally by DiffLens — AI-assisted review only, not legal advice.",
    st_added: "added",
    st_removed: "removed",
    st_modified: "modified",
    st_moved: "moved",
    st_equal: "unchanged",
    kind_added: "Added",
    kind_removed: "Removed",
    kind_modified: "Modified",
    kind_moved: "Moved",
    kind_equal: "Unchanged",
    sev_info: "Info",
    sev_low: "Low",
    sev_medium: "Medium",
    sev_high: "High",
    sev_critical: "Critical",
};

/// Vietnamese label table.
static VI_LABELS: Labels = Labels {
    lang: ReportLang::Vi,
    title: "Báo cáo so sánh tài liệu",
    old_file: "Tệp cũ",
    new_file: "Tệp mới",
    generated: "Tạo lúc",
    format: "Định dạng",
    alignment_on: "Ghép cặp ngữ nghĩa: bật",
    alignment_off: "Ghép cặp ngữ nghĩa: tắt (dự phòng theo vị trí)",
    ocr_note: "Bao gồm các trang trích xuất bằng OCR (thử nghiệm)",
    session_id: "Phiên",
    summary: "Tổng quan",
    changes: "Các thay đổi",
    no_changes: "Không có khác biệt — hai tài liệu trùng nhau.",
    audit: "Lịch sử kiểm định",
    audit_note: "Các quy tắc được áp dụng theo thứ tự; mức cao nhất được chọn.",
    change: "Thay đổi",
    rule: "Quy tắc",
    old_value: "Bản cũ",
    new_value: "Bản mới",
    delta: "Chênh lệch",
    detail: "Chi tiết",
    page: "Trang",
    similarity: "khớp",
    footer: "Được tạo cục bộ bởi DiffLens — chỉ hỗ trợ rà soát bằng AI, không phải tư vấn pháp lý.",
    st_added: "thêm",
    st_removed: "xóa",
    st_modified: "sửa",
    st_moved: "dời",
    st_equal: "không đổi",
    kind_added: "Thêm mới",
    kind_removed: "Đã xóa",
    kind_modified: "Đã sửa",
    kind_moved: "Đã di chuyển",
    kind_equal: "Không đổi",
    sev_info: "Thông tin",
    sev_low: "Thấp",
    sev_medium: "Trung bình",
    sev_high: "Cao",
    sev_critical: "Nghiêm trọng",
};

/// Label table for `lang`.
pub fn labels(lang: ReportLang) -> &'static Labels {
    match lang {
        ReportLang::En => &EN_LABELS,
        ReportLang::Vi => &VI_LABELS,
    }
}

impl Labels {
    /// Kind display label.
    pub fn kind(&self, kind: ChangeKind) -> &'static str {
        match kind {
            ChangeKind::Added => self.kind_added,
            ChangeKind::Removed => self.kind_removed,
            ChangeKind::Modified => self.kind_modified,
            ChangeKind::Moved => self.kind_moved,
            ChangeKind::Equal => self.kind_equal,
        }
    }

    /// Severity display label.
    pub fn severity(&self, severity: Severity) -> &'static str {
        match severity {
            Severity::Info => self.sev_info,
            Severity::Low => self.sev_low,
            Severity::Medium => self.sev_medium,
            Severity::High => self.sev_high,
            Severity::Critical => self.sev_critical,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_formats_known_values() {
        assert_eq!(format_utc_ms(0), "1970-01-01 00:00:00.000 UTC");
        // 2026-11-18 12:34:56.789 UTC
        assert_eq!(
            format_utc_ms(1_795_005_296_789),
            "2026-11-18 12:34:56.789 UTC"
        );
        // Leap year: 2024-02-29 00:00:00.000 UTC
        assert_eq!(
            format_utc_ms(1_709_164_800_000),
            "2024-02-29 00:00:00.000 UTC"
        );
    }

    #[test]
    fn lang_tag_parsing() {
        assert_eq!(ReportLang::from_tag("en"), ReportLang::En);
        assert_eq!(ReportLang::from_tag("en-US"), ReportLang::En);
        assert_eq!(ReportLang::from_tag("vi"), ReportLang::Vi);
        assert_eq!(ReportLang::from_tag("vi-VN"), ReportLang::Vi);
        assert_eq!(ReportLang::from_tag("VI"), ReportLang::Vi);
        assert_eq!(ReportLang::from_tag("fr"), ReportLang::En);
    }

    #[test]
    fn every_flag_key_has_non_empty_labels_in_both_languages() {
        for key in all_report_flag_keys() {
            for lang in [ReportLang::En, ReportLang::Vi] {
                let label = flag_label(labels(lang), key);
                assert!(!label.is_empty(), "key {key} lang {lang:?} empty");
                assert!(!label.contains("flag."), "raw key leaked for {key}");
            }
        }
    }
}
