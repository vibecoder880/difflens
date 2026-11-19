//! Deterministic entity extraction over block text (VN + EN).
//!
//! Extractors return normalized values plus byte offsets into the
//! (NFC-normalized) block text. All regexes are precompiled once
//! ([`std::sync::OnceLock`]) and matching is ordered, so the same input
//! always yields the same entity list. Malformed candidates are dropped —
//! extraction never panics.
//!
//! ## Number-separator disambiguation
//!
//! Vietnamese writes thousands with dots (`1.500.000₫`) and decimals with a
//! comma (`1,5`); English writes thousands with commas (`1,500,000`) and
//! decimals with a dot (`$1,500.50`). One separator in isolation is
//! ambiguous (`1.000` / `1,000`). Rules applied to each digit token:
//!
//! 1. **Both separators present**: the first one occurring is the group
//!    separator, the last one is the decimal separator
//!    (`1.234,56` → 1234.56, `1,234.56` → 1234.56).
//! 2. **One separator kind, multiple occurrences**: all are group
//!    separators (`1.500.000` → 1500000, groups must be 3 digits wide).
//! 3. **Exactly one occurrence, followed by exactly 3 digits**: group
//!    separator — the conservative integer reading for money-style
//!    formatting (`990.000đ` → 990000, `1,500` → 1500) — unless the head is
//!    only zeros, which cannot be a thousands group (`0,075` → 0.075).
//! 4. **Exactly one occurrence, followed by 1–2 digits (or other widths)**:
//!    decimal separator (`1,5` → 1.5, `0.5` → 0.5).
//! 5. **Any other shape** (empty groups, non-3-wide groups, trailing
//!    garbage) fails validation and the candidate is dropped.
//!
//! Money additionally requires an explicit currency token or a Vietnamese
//! scale word (`triệu`/`tỷ`/`nghìn`/`ngàn`/`k`); bare numbers are ignored
//! (the documented false-positive guard from the phase plan). A scale word
//! with no currency token defaults to VND.

use std::sync::OnceLock;

use regex::{Captures, Regex};
use unicode_normalization::UnicodeNormalization;

/// Currency of a normalized money amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    Vnd,
    Usd,
    Eur,
}

impl Currency {
    /// Stable code used in `detail` fields.
    pub fn code(self) -> &'static str {
        match self {
            Currency::Vnd => "VND",
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
        }
    }
}

/// Kind of an extracted entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityKind {
    Money,
    Date,
    Duration,
    Percent,
    Reference,
}

impl EntityKind {
    /// Stable lowercase name used in `detail` fields.
    pub fn name(self) -> &'static str {
        match self {
            EntityKind::Money => "money",
            EntityKind::Date => "date",
            EntityKind::Duration => "duration",
            EntityKind::Percent => "percent",
            EntityKind::Reference => "reference",
        }
    }
}

/// Canonical parsed value of an entity.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityValue {
    /// `amount` is the scaled value (`500 triệu` → 5e8); `scale_note` keeps
    /// the raw scale word when one was used.
    Money {
        amount: f64,
        currency: Currency,
        scale_note: Option<&'static str>,
    },
    /// `iso` is `yyyy-mm-dd`; `day_number` is a proleptic-Gregorian day
    /// count (civil-days algorithm) so day deltas are plain subtraction.
    Date {
        iso: String,
        day_number: i64,
    },
    /// Days-equivalent value (`tháng` → ×30, `tuần` → ×7, `năm` → ×365,
    /// working/business days counted at face value). `unit` preserves the
    /// source unit for display and audit.
    Duration {
        days_equivalent: f64,
        unit: &'static str,
    },
    Percent {
        value: f64,
    },
    /// Uppercased, trailing-punctuation-trimmed document reference.
    Reference {
        normalized: String,
    },
}

impl EntityValue {
    /// Numeric key used for closest-by-value pairing. References pair by
    /// their normalized string instead (see [`crate::heuristic::deltas`]),
    /// so they report 0.0 here.
    pub fn numeric_key(&self) -> f64 {
        match self {
            EntityValue::Money { amount, .. } => *amount,
            EntityValue::Date { day_number, .. } => *day_number as f64,
            EntityValue::Duration {
                days_equivalent, ..
            } => *days_equivalent,
            EntityValue::Percent { value } => *value,
            EntityValue::Reference { .. } => 0.0,
        }
    }

    /// Equality of canonical values (references compare by normalized text;
    /// floats compare exactly — values come from the same parser, so equal
    /// formatting always yields bit-equal values).
    pub fn same_value(&self, other: &EntityValue) -> bool {
        match (self, other) {
            (
                EntityValue::Money {
                    amount: a,
                    currency: ca,
                    ..
                },
                EntityValue::Money {
                    amount: b,
                    currency: cb,
                    ..
                },
            ) => a == b && ca == cb,
            (EntityValue::Date { day_number: a, .. }, EntityValue::Date { day_number: b, .. }) => {
                a == b
            }
            (
                EntityValue::Duration {
                    days_equivalent: a,
                    unit: ua,
                },
                EntityValue::Duration {
                    days_equivalent: b,
                    unit: ub,
                },
            ) => a == b && ua == ub,
            (EntityValue::Percent { value: a }, EntityValue::Percent { value: b }) => a == b,
            (
                EntityValue::Reference { normalized: a },
                EntityValue::Reference { normalized: b },
            ) => a == b,
            _ => false,
        }
    }
}

/// One extracted entity with offsets into the normalized text.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub kind: EntityKind,
    /// Raw matched text (as found in the normalized input).
    pub text: String,
    /// Byte span `(start, end)` into the normalized text.
    pub span: (usize, usize),
    pub value: EntityValue,
}

/// Extract all entities from `raw` text, ordered by byte span.
///
/// The input is NFC-normalized first (Vietnamese text may arrive in NFD
/// from some extractors); offsets refer to the normalized text.
pub fn extract_entities(raw: &str) -> Vec<Entity> {
    let text: String = raw.nfc().collect();
    let mut occupied: Vec<(usize, usize)> = Vec::new();
    let mut entities: Vec<Entity> = Vec::new();
    // Fixed processing order gives deterministic ownership of overlapping
    // text (`ngày 15 tháng 3 năm 2026` is a date, never a duration;
    // `990.000đ` is money, never a bare number).
    collect_money(&text, &mut occupied, &mut entities);
    collect_dates(&text, &mut occupied, &mut entities);
    collect_durations(&text, &mut occupied, &mut entities);
    collect_percents(&text, &mut occupied, &mut entities);
    collect_references(&text, &mut occupied, &mut entities);
    entities.sort_by_key(|e| e.span);
    entities
}

/// True when `[start, end)` overlaps any occupied range.
fn overlaps(occupied: &[(usize, usize)], start: usize, end: usize) -> bool {
    occupied.iter().any(|(s, e)| start < *e && *s < end)
}

/// Skip empty zero-width matches (defensive: none of the patterns below can
/// produce them, but guard anyway so the collectors never loop or panic).
fn whole_match<'a>(caps: &'a Captures<'a>) -> Option<regex::Match<'a>> {
    caps.get(0).filter(|m| !m.as_str().is_empty())
}

// ---------------------------------------------------------------------------
// Number parsing with the documented disambiguation rules
// ---------------------------------------------------------------------------

/// Group validity: `123` or `d{1,3}(sep ddd)+`.
fn groups_valid(int_part: &str, sep: char) -> bool {
    if !int_part.contains(sep) {
        return !int_part.is_empty();
    }
    let mut groups = int_part.split(sep);
    let head = match groups.next() {
        Some(h) => h,
        None => return false,
    };
    if head.is_empty() || head.len() > 3 || !head.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    groups.all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
}

/// Parse a digit+separator token into `f64`, per the module disambiguation
/// rules. Returns `None` when the shape is invalid — callers drop the
/// candidate.
pub fn parse_number(token: &str) -> Option<f64> {
    let has_dot = token.contains('.');
    let has_comma = token.contains(',');
    let int_digits;
    let mut frac: Option<String> = None;
    match (has_dot, has_comma) {
        (true, true) => {
            let (group, dec) = if token.find('.') < token.find(',') {
                ('.', ',')
            } else {
                (',', '.')
            };
            let last_dec = token.rfind(dec)?;
            // Integer part = everything before the decimal separator.
            let ip = &token[..last_dec];
            let fp = &token[last_dec + 1..];
            if !groups_valid(ip, group) {
                return None;
            }
            int_digits = ip.replace(group, "");
            if fp.is_empty() || !fp.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            frac = Some(fp.to_string());
        }
        (true, false) | (false, true) => {
            let sep = if has_dot { '.' } else { ',' };
            if token.matches(sep).count() > 1 {
                if !groups_valid(token, sep) {
                    return None;
                }
                int_digits = token.replace(sep, "");
            } else {
                let pos = token.find(sep)?;
                let after = &token[pos + 1..];
                if after.len() == 3 && after.bytes().all(|b| b.is_ascii_digit()) {
                    // Rule 3: conservative integer reading (`990.000` → 990000) —
                    // except when the head is only zeros: a grouped integer never
                    // starts with a zero-only group, so `0,075` / `0.075` are the
                    // decimal 0.075 (Vietnamese small percentages like `0,075%`
                    // used to be misread as 75 and inflate deltas ~×3000).
                    let head = &token[..pos];
                    if head.is_empty() || !head.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    if head.bytes().all(|b| b == b'0') {
                        int_digits = head.to_string();
                        frac = Some(after.to_string());
                    } else {
                        int_digits = format!("{head}{after}");
                    }
                } else {
                    // Rule 4: decimal separator.
                    let head = &token[..pos];
                    if head.is_empty() || !head.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    if after.is_empty() || !after.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    int_digits = head.to_string();
                    frac = Some(after.to_string());
                }
            }
        }
        (false, false) => {
            int_digits = token.to_string();
            if int_digits.is_empty() || !int_digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
        }
    }
    if int_digits.is_empty() || !int_digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut value: f64 = int_digits.parse().ok()?;
    if let Some(f) = frac {
        value += f.parse::<f64>().ok()? / 10f64.powi(f.len() as i32);
    }
    Some(value)
}

/// Trim trailing separators left by the greedy digit token
/// (`"2.000.000."` at sentence end → `"2.000.000"`).
fn trim_num_token(token: &str) -> &str {
    token.trim_end_matches(['.', ','])
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// Guard against matches glued to surrounding words (`500km`, `abc100`).
/// `has_prefix_cur` relaxes the left check for `US$100`.
fn boundary_ok(text: &str, start: usize, end: usize, relax_left: bool) -> bool {
    let bytes = text.as_bytes();
    if start > 0 && !relax_left && is_word_byte(bytes[start - 1]) {
        return false;
    }
    if end < bytes.len() && is_word_byte(bytes[end]) {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Money
// ---------------------------------------------------------------------------

fn money_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Alternation order matters (leftmost-first): `đồng` before `đ`,
        // `VNĐ` before `Đ`, `US$` before `$`. The trailing `\s*` lives
        // INSIDE the optional pre group so that, when no currency prefix
        // matches, the match starts exactly at the first digit (a leading
        // `\s*` outside would be absorbed into the span and then trip the
        // left word-boundary guard on text like `phạt 5.000.000₫`).
        Regex::new(
            "(?i)(?:(?P<pre>US\\$|USD|\\$|EUR|€)\\s*)?(?P<num>\\d[\\d.,]*)\\s*\
             (?P<scale>triệu|tỷ|nghìn|ngàn|[kK])?\\s*\
             (?P<cur>₫|VNĐ|đồng|VND|đ|USD|\\$|EUR|€)?",
        )
        .expect("money regex")
    })
}

fn scale_factor(word: &str) -> Option<(f64, &'static str)> {
    match word {
        "k" | "K" => Some((1e3, "k")),
        "nghìn" | "ngàn" => Some((1e3, "nghìn")),
        "triệu" => Some((1e6, "triệu")),
        "tỷ" => Some((1e9, "tỷ")),
        _ => None,
    }
}

fn currency_of(token: &str) -> Option<Currency> {
    Some(match token {
        "$" | "US$" | "usd" | "USD" => Currency::Usd,
        "€" | "eur" | "EUR" => Currency::Eur,
        // Remaining hits are the VND tokens (case-insensitive regex).
        _ => Currency::Vnd,
    })
}

fn collect_money(text: &str, occupied: &mut Vec<(usize, usize)>, out: &mut Vec<Entity>) {
    for caps in money_regex().captures_iter(text) {
        let Some(whole) = whole_match(&caps) else {
            continue;
        };
        let Some(num) = caps.name("num") else {
            continue;
        };
        let pre = caps.name("pre").filter(|m| !m.as_str().is_empty());
        let scale = caps.name("scale").filter(|m| !m.as_str().is_empty());
        let cur = caps.name("cur").filter(|m| !m.as_str().is_empty());
        // Require an explicit currency or scale word: bare numbers ignored.
        if pre.is_none() && scale.is_none() && cur.is_none() {
            continue;
        }
        // The `\s*` between the scale word and the (optional) currency token
        // is absorbed into the whole match even when no currency follows, so
        // `whole.end()` can trail the content by whitespace — and the right
        // word-boundary guard would then reject `2 tỷ mỗi tháng`. End the
        // span at the last content group instead.
        let content_end = cur
            .map(|m| m.end())
            .or_else(|| scale.map(|m| m.end()))
            .unwrap_or_else(|| num.end());
        if !boundary_ok(text, whole.start(), content_end, pre.is_some()) {
            continue;
        }
        let Some(amount_num) = parse_number(trim_num_token(num.as_str())) else {
            continue;
        };
        let mut amount = amount_num;
        let mut scale_note = None;
        if let Some(s) = scale
            && let Some((factor, note)) = scale_factor(s.as_str())
        {
            amount *= factor;
            scale_note = Some(note);
        }
        let currency = pre
            .and_then(|m| currency_of(m.as_str()))
            .or_else(|| cur.and_then(|m| currency_of(m.as_str())))
            // Scale word with no currency token defaults to VND.
            .unwrap_or(Currency::Vnd);
        out.push(Entity {
            kind: EntityKind::Money,
            text: text[whole.start()..content_end].to_string(),
            span: (whole.start(), content_end),
            value: EntityValue::Money {
                amount,
                currency,
                scale_note,
            },
        });
        occupied.push((whole.start(), content_end));
    }
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

fn date_patterns() -> &'static [Regex] {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    RES.get_or_init(|| {
        [
            // ISO first: `2026-03-15`.
            r"(?P<y>\d{4})-(?P<m>\d{2})-(?P<d>\d{2})",
            // Vietnamese words: `ngày 15 tháng 3 năm 2026`.
            r"ngày\s+(?P<d>\d{1,2})\s+tháng\s+(?P<m>\d{1,2})\s+năm\s+(?P<y>\d{4})",
            // English month names, both orders.
            r"(?i)(?P<mon>january|february|march|april|may|june|july|august|september|october|november|december)\.?\s+(?P<d>\d{1,2})(?:st|nd|rd|th)?,\s*(?P<y>\d{4})",
            r"(?i)(?P<d>\d{1,2})(?:st|nd|rd|th)?\s+(?P<mon>january|february|march|april|may|june|july|august|september|october|november|december)\.?,?\s+(?P<y>\d{4})",
            // Slash day-first: `15/03/2026`.
            r"(?P<d>\d{1,2})/(?P<m>\d{1,2})/(?P<y>\d{4})",
            // Dashed day-first: `15-03-2026` (ISO already claimed above).
            r"(?P<d>\d{1,2})-(?P<m>\d{1,2})-(?P<y>\d{4})",
        ]
        .into_iter()
        .map(|p| Regex::new(p).expect("date regex"))
        .collect()
    })
}

fn english_month(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    Some(match n.as_str() {
        "january" => 1,
        "february" => 2,
        "march" => 3,
        "april" => 4,
        "may" => 5,
        "june" => 6,
        "july" => 7,
        "august" => 8,
        "september" => 9,
        "october" => 10,
        "november" => 11,
        "december" => 12,
        _ => return None,
    })
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic-Gregorian date (Howard Hinnant's
/// `days_from_civil`). Callers validate ranges first.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // 0..=399
    let mp = i64::from((m + 9) % 12); // Mar=0..Feb=11
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Validate a (y, m, d) triple and produce the normalized value.
fn make_date(y: i64, m: u32, d: u32) -> Option<EntityValue> {
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(EntityValue::Date {
        iso: format!("{y:04}-{m:02}-{d:02}"),
        day_number: days_from_civil(y, m, d),
    })
}

/// Day-first default (VN convention); swapped only when the month slot
/// forces it (`03/15/2026` — a value > 12 in the second field must be the
/// day). `2026-03-15` ISO never reaches here.
fn split_dmy(d_raw: u32, m_raw: u32) -> (u32, u32) {
    if m_raw > 12 && d_raw <= 12 {
        (m_raw, d_raw)
    } else {
        (d_raw, m_raw)
    }
}

fn collect_dates(text: &str, occupied: &mut Vec<(usize, usize)>, out: &mut Vec<Entity>) {
    for re in date_patterns() {
        for caps in re.captures_iter(text) {
            let Some(whole) = whole_match(&caps) else {
                continue;
            };
            if overlaps(occupied, whole.start(), whole.end()) {
                continue;
            }
            let group = |name: &str| caps.name(name).map(|m| m.as_str());
            let parsed = if let Some(mon) = group("mon") {
                let Some(month) = english_month(mon) else {
                    continue;
                };
                let Some(d) = group("d").and_then(|s| s.parse::<u32>().ok()) else {
                    continue;
                };
                let Some(y) = group("y").and_then(|s| s.parse::<i64>().ok()) else {
                    continue;
                };
                make_date(y, month, d)
            } else {
                let (Some(d_raw), Some(m_raw), Some(y)) = (
                    group("d").and_then(|s| s.parse::<u32>().ok()),
                    group("m").and_then(|s| s.parse::<u32>().ok()),
                    group("y").and_then(|s| s.parse::<i64>().ok()),
                ) else {
                    continue;
                };
                let (d, m) = split_dmy(d_raw, m_raw);
                make_date(y, m, d)
            };
            if let Some(value) = parsed {
                out.push(Entity {
                    kind: EntityKind::Date,
                    text: whole.as_str().to_string(),
                    span: (whole.start(), whole.end()),
                    value,
                });
                occupied.push((whole.start(), whole.end()));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Durations
// ---------------------------------------------------------------------------

fn duration_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Longest unit alternatives first (`ngày làm việc` before `ngày`).
        Regex::new(
            "(?i)(?:((?:within|trong\\s*vòng|trong|sau|after)\\s+))?\
             (?P<num>\\d+(?:[.,]\\d+)?)\\s+\
             (?P<unit>ngày\\s+làm\\s+việc|working\\s+days?|business\\s+days?|\
             days?|ngày|weeks?|tuần|months?|tháng|years?|năm|quarters?|quý)",
        )
        .expect("duration regex")
    })
}

fn unit_factor(unit: &str) -> Option<(f64, &'static str)> {
    let u = unit.to_lowercase();
    let compact = u.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(match compact.as_str() {
        "ngày" | "day" | "days" => (1.0, "days"),
        "ngày làm việc" | "working day" | "working days" | "business day" | "business days" => {
            (1.0, "working_days")
        }
        "tuần" | "week" | "weeks" => (7.0, "weeks"),
        "tháng" | "month" | "months" => (30.0, "months"),
        "quý" | "quarter" | "quarters" => (90.0, "quarters"),
        "năm" | "year" | "years" => (365.0, "years"),
        _ => return None,
    })
}

fn collect_durations(text: &str, occupied: &mut Vec<(usize, usize)>, out: &mut Vec<Entity>) {
    for caps in duration_regex().captures_iter(text) {
        let Some(whole) = whole_match(&caps) else {
            continue;
        };
        if overlaps(occupied, whole.start(), whole.end()) {
            continue;
        }
        if !boundary_ok(text, whole.start(), whole.end(), false) {
            continue;
        }
        let Some(unit_raw) = caps.name("unit").map(|m| m.as_str()) else {
            continue;
        };
        let Some((factor, unit_name)) = unit_factor(unit_raw) else {
            continue;
        };
        let Some(num) = caps
            .name("num")
            .and_then(|m| parse_number(trim_num_token(m.as_str())))
        else {
            continue;
        };
        out.push(Entity {
            kind: EntityKind::Duration,
            text: whole.as_str().to_string(),
            span: (whole.start(), whole.end()),
            value: EntityValue::Duration {
                days_equivalent: num * factor,
                unit: unit_name,
            },
        });
        occupied.push((whole.start(), whole.end()));
    }
}

// ---------------------------------------------------------------------------
// Percent
// ---------------------------------------------------------------------------

fn percent_numeric_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?P<num>\d+(?:[.,]\d+)?)\s?%").expect("percent regex"))
}

fn percent_word_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Word percents (`five percent`, `half percent`). Boundaries are
        // approximated with adjacent character classes (the regex crate has
        // no lookaround); the entity span covers only the phrase itself.
        Regex::new(
            "(?i)(?:\\A|[^A-Za-z0-9])(?P<word>one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|half)\\s+(?P<phrase>percent)(?:\\z|[^A-Za-z0-9])",
        )
        .expect("percent word regex")
    })
}

fn word_percent(word: &str) -> Option<f64> {
    Some(match word.to_ascii_lowercase().as_str() {
        "half" => 50.0,
        "one" => 1.0,
        "two" => 2.0,
        "three" => 3.0,
        "four" => 4.0,
        "five" => 5.0,
        "six" => 6.0,
        "seven" => 7.0,
        "eight" => 8.0,
        "nine" => 9.0,
        "ten" => 10.0,
        "eleven" => 11.0,
        "twelve" => 12.0,
        _ => return None,
    })
}

fn collect_percents(text: &str, occupied: &mut Vec<(usize, usize)>, out: &mut Vec<Entity>) {
    for caps in percent_numeric_regex().captures_iter(text) {
        let Some(whole) = whole_match(&caps) else {
            continue;
        };
        if overlaps(occupied, whole.start(), whole.end()) {
            continue;
        }
        let Some(value) = caps.name("num").and_then(|m| parse_number(m.as_str())) else {
            continue;
        };
        out.push(Entity {
            kind: EntityKind::Percent,
            text: whole.as_str().to_string(),
            span: (whole.start(), whole.end()),
            value: EntityValue::Percent { value },
        });
        occupied.push((whole.start(), whole.end()));
    }
    for caps in percent_word_regex().captures_iter(text) {
        let Some(word) = caps.name("word") else {
            continue;
        };
        let Some(phrase) = caps.name("phrase") else {
            continue;
        };
        let (start, end) = (word.start(), phrase.end());
        if overlaps(occupied, start, end) {
            continue;
        }
        let Some(value) = word_percent(word.as_str()) else {
            continue;
        };
        out.push(Entity {
            kind: EntityKind::Percent,
            text: text[start..end].to_string(),
            span: (start, end),
            value: EntityValue::Percent { value },
        });
        occupied.push((start, end));
    }
}

// ---------------------------------------------------------------------------
// References (số hợp đồng / tài liệu)
// ---------------------------------------------------------------------------

fn reference_regexes() -> &'static [Regex] {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    RES.get_or_init(|| {
        [
            // `Contract No. 45/2026/NDA`, `số hợp đồng 123/2026/HD`.
            "(?i)(?:số\\s+(?:hợp\\s+đồng|tài\\s+liệu|tham\\s+chiếu)|contract\\s+no\\.?|document\\s+no\\.?|ref(?:erence)?\\s+no\\.?)\\s*:?\\s*(?P<ref>[0-9][0-9A-Za-z/.\\-]{2,40})",
            // Bare `số:` / `No.` only accept fully structured ids
            // (`số: 123/2026/HD`) to avoid false positives on prose
            // (`số 500 triệu`).
            "(?i)(?:số|no)\\.?\\s*:??\\s*(?P<ref>[0-9]+/[0-9]{4}/[0-9A-Za-z\\-]+)",
        ]
        .into_iter()
        .map(|p| Regex::new(p).expect("reference regex"))
        .collect()
    })
}

fn collect_references(text: &str, occupied: &mut Vec<(usize, usize)>, out: &mut Vec<Entity>) {
    for re in reference_regexes() {
        for caps in re.captures_iter(text) {
            let Some(whole) = whole_match(&caps) else {
                continue;
            };
            if overlaps(occupied, whole.start(), whole.end()) {
                continue;
            }
            let Some(ref_cap) = caps.name("ref") else {
                continue;
            };
            let normalized = ref_cap
                .as_str()
                .trim_matches(|c: char| c == '.' || c == ',')
                .to_uppercase();
            if normalized.len() < 3 {
                continue;
            }
            out.push(Entity {
                kind: EntityKind::Reference,
                text: whole.as_str().to_string(),
                span: (whole.start(), whole.end()),
                value: EntityValue::Reference { normalized },
            });
            occupied.push((whole.start(), whole.end()));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests: extractor unit tables (VN + EN, incl. disambiguation cases)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(EntityKind, String)> {
        extract_entities(text)
            .into_iter()
            .map(|e| (e.kind, e.text))
            .collect()
    }

    fn money(text: &str) -> Vec<Entity> {
        extract_entities(text)
            .into_iter()
            .filter(|e| e.kind == EntityKind::Money)
            .collect()
    }

    fn first_money_amount(text: &str) -> f64 {
        match money(text).first().expect("money entity").value {
            EntityValue::Money { amount, .. } => amount,
            _ => panic!("not money"),
        }
    }

    // ---- money VN ----

    #[test]
    fn money_vn_dot_thousands() {
        assert!((first_money_amount("Phí là 1.500.000₫ mỗi tháng.") - 1_500_000.0).abs() < 1e-6);
        assert!((first_money_amount("990.000đ") - 990_000.0).abs() < 1e-6);
        assert_eq!(
            money("1.500.000 VNĐ")[0].value,
            EntityValue::Money {
                amount: 1_500_000.0,
                currency: Currency::Vnd,
                scale_note: None,
            }
        );
    }

    #[test]
    fn money_vn_scale_words() {
        assert!((first_money_amount("500 triệu đồng") - 5e8).abs() < 1.0);
        assert!((first_money_amount("phí 2 tỷ") - 2e9).abs() < 1.0);
        assert!((first_money_amount("trả 500k trước") - 5e5).abs() < 1.0);
        let e = &money("500 triệu")[0];
        match &e.value {
            EntityValue::Money {
                currency,
                scale_note,
                ..
            } => {
                assert_eq!(*currency, Currency::Vnd, "scale defaults to VND");
                assert_eq!(*scale_note, Some("triệu"));
            }
            _ => panic!("not money"),
        }
    }

    // ---- money EN + ambiguity ----

    #[test]
    fn money_en_comma_thousands() {
        assert!((first_money_amount("$1,500.50") - 1500.5).abs() < 1e-6);
        assert!((first_money_amount("1,500,000 VND") - 1_500_000.0).abs() < 1e-6);
        assert!((first_money_amount("paid 1.234,56 EUR") - 1234.56).abs() < 1e-6);
    }

    #[test]
    fn money_disambiguation_rules() {
        // Rule 3: single separator + exactly 3 digits = thousands.
        assert!((first_money_amount("990.000đ") - 990_000.0).abs() < 1e-6);
        // Rule 4: single separator + 1-2 digits = decimal.
        let e = &money("$1.5")[0];
        match e.value {
            EntityValue::Money { amount, .. } => assert!((amount - 1.5).abs() < 1e-9),
            _ => panic!(),
        }
        // Rule 1: both separators, first = thousands, last = decimal.
        assert!((first_money_amount("$1,234.56") - 1234.56).abs() < 1e-6);
        // Rule 5: invalid group widths dropped.
        assert!(money("1.23.456 VND").is_empty());
        // Bare numbers without currency/scale are ignored.
        assert!(money("tổng 1.500.000 không rõ").is_empty());
        // `500km` is not money (boundary guard).
        assert!(money("đi 500km").is_empty());
    }

    // ---- dates ----

    #[test]
    fn dates_all_formats_normalize_to_iso() {
        let cases = [
            ("hạn 15/03/2026", "2026-03-15"),
            ("hạn 2026-03-15", "2026-03-15"),
            ("ngày 15 tháng 3 năm 2026", "2026-03-15"),
            ("due March 15, 2026", "2026-03-15"),
            ("due 15 March 2026", "2026-03-15"),
            ("hạn 15-03-2026", "2026-03-15"),
        ];
        for (text, iso) in cases {
            let e = extract_entities(text)
                .into_iter()
                .find(|e| e.kind == EntityKind::Date)
                .unwrap_or_else(|| panic!("no date in {text}"));
            match e.value {
                EntityValue::Date { iso: got, .. } => assert_eq!(got, iso, "input: {text}"),
                _ => panic!("not a date"),
            }
        }
    }

    #[test]
    fn date_us_order_forced_by_month_slot() {
        let e = extract_entities("dated 03/15/2026")
            .into_iter()
            .find(|e| e.kind == EntityKind::Date)
            .expect("date");
        match e.value {
            EntityValue::Date { iso, .. } => assert_eq!(iso, "2026-03-15"),
            _ => panic!(),
        }
    }

    #[test]
    fn invalid_dates_dropped() {
        assert!(
            extract_entities("ngày 30 tháng 2 năm 2026")
                .into_iter()
                .find(|e| e.kind == EntityKind::Date)
                .is_none()
        );
        assert!(extract_entities("32/13/2026").is_empty());
    }

    #[test]
    fn day_number_is_days_since_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(
            days_from_civil(2026, 3, 15) - days_from_civil(2025, 3, 15),
            365
        );
        assert_eq!(
            days_from_civil(2027, 3, 15) - days_from_civil(2026, 3, 15),
            365
        );
        assert_eq!(
            days_from_civil(2100, 3, 15) - days_from_civil(2099, 3, 15),
            365
        );
    }

    // ---- durations ----

    #[test]
    fn durations_normalize_to_day_equivalents() {
        let get = |text: &str| match extract_entities(text)
            .into_iter()
            .find(|e| e.kind == EntityKind::Duration)
            .unwrap_or_else(|| panic!("no duration in {text}"))
            .value
        {
            EntityValue::Duration {
                days_equivalent,
                unit,
            } => (days_equivalent, unit),
            _ => panic!(),
        };
        assert_eq!(get("trong 30 ngày"), (30.0, "days"));
        assert_eq!(get("within 30 days"), (30.0, "days"));
        assert_eq!(get("60 ngày làm việc"), (60.0, "working_days"));
        assert_eq!(get("3 tháng"), (90.0, "months"));
        assert_eq!(get("12 tháng"), (360.0, "months"));
        assert_eq!(get("2 tuần"), (14.0, "weeks"));
        assert_eq!(get("after 1 year"), (365.0, "years"));
    }

    #[test]
    fn date_phrase_is_not_a_duration() {
        // The `ngày … tháng … năm …` span must stay a single date.
        let es = extract_entities("hạn cuối ngày 15 tháng 3 năm 2026");
        assert!(es.iter().all(|e| e.kind != EntityKind::Duration));
        assert_eq!(es.iter().filter(|e| e.kind == EntityKind::Date).count(), 1);
    }

    // ---- percent ----

    #[test]
    fn percent_numeric_and_words() {
        let get = |text: &str| match extract_entities(text)
            .into_iter()
            .find(|e| e.kind == EntityKind::Percent)
            .unwrap_or_else(|| panic!("no percent in {text}"))
            .value
        {
            EntityValue::Percent { value } => value,
            _ => panic!(),
        };
        assert!((get("lãi suất 5%") - 5.0).abs() < 1e-9);
        assert!((get("chiết khấu 5,5 %") - 5.5).abs() < 1e-9);
        assert!((get("0.5% phí") - 0.5).abs() < 1e-9);
        assert!((get("increase of five percent") - 5.0).abs() < 1e-9);
        assert!((get("half percent") - 50.0).abs() < 1e-9);
        // `1990%` style still parses as a percent (numeric + `%`).
        assert!((get("tăng 1990%") - 1990.0).abs() < 1e-9);
    }

    #[test]
    fn zero_head_three_digit_tail_is_decimal_not_thousands() {
        // Regression (QA hands-on v1.0.2): a zero-only head cannot be a
        // thousands group, so `0,075` / `0.075` are the decimal 0.075.
        // They used to parse as 75 via the rule-3 integer reading.
        assert!((parse_number("0,075").unwrap() - 0.075).abs() < 1e-12);
        assert!((parse_number("0.075").unwrap() - 0.075).abs() < 1e-12);
        assert!((parse_number("0,500").unwrap() - 0.5).abs() < 1e-12);
        // Non-zero heads keep the conservative integer reading.
        assert!((parse_number("990.000").unwrap() - 990_000.0).abs() < 1e-9);
        assert!((parse_number("1,500").unwrap() - 1500.0).abs() < 1e-9);
        assert!((parse_number("1.500.000").unwrap() - 1_500_000.0).abs() < 1e-9);
    }

    #[test]
    fn vietnamese_small_percent_contract_value() {
        // `phạt 0,05%/ngày → 0,075%/ngày` must yield a relative delta of
        // +0.5 (+50%), not +1499 (the old 75.0 misparse).
        let (flags, _) = crate::heuristic::enrich_texts(
            "Mỗi chậm thanh toán bị phạt 0,05%/ngày.",
            "Mỗi chậm thanh toán bị phạt 0,075%/ngày.",
        );
        let pct = flags
            .iter()
            .find(|f| f.rule_id == crate::heuristic::deltas::RULE_PERCENT_CHANGED)
            .expect("PERCENT_CHANGED fired");
        assert!((pct.delta.unwrap() - 0.5).abs() < 1e-9, "delta {:?}", pct.delta);
        let detail = pct.detail.as_deref().unwrap();
        assert!(detail.starts_with("0.05%→"), "detail {detail:?}");
        assert!(detail.contains("pp"), "detail {detail:?}");
    }

    // ---- references ----

    #[test]
    fn references_extract_structured_ids() {
        let get = |text: &str| match extract_entities(text)
            .into_iter()
            .find(|e| e.kind == EntityKind::Reference)
            .unwrap_or_else(|| panic!("no reference in {text}"))
            .value
        {
            EntityValue::Reference { normalized } => normalized,
            _ => panic!(),
        };
        assert_eq!(get("số: 123/2026/HD"), "123/2026/HD");
        assert_eq!(get("Contract No. 45/2026/NDA-01"), "45/2026/NDA-01");
        // Prose near `số` without a structured id does not fire.
        assert!(
            extract_entities("số tiền 500 triệu đồng")
                .into_iter()
                .find(|e| e.kind == EntityKind::Reference)
                .is_none()
        );
    }

    // ---- ordering + offsets ----

    #[test]
    fn entities_ordered_by_span_and_offsets_valid() {
        let text = "Phí 1.500.000₫, hạn 15/03/2026, lãi 5%.";
        let es = extract_entities(text);
        let spans: Vec<(usize, usize)> = es.iter().map(|e| e.span).collect();
        assert!(spans.windows(2).all(|w| w[0].0 <= w[1].0));
        for e in &es {
            assert_eq!(&text[e.span.0..e.span.1], e.text);
        }
    }

    #[test]
    fn kinds_summary() {
        assert_eq!(
            kinds("1.500.000₫ 15/03/2026 30 ngày 5% số: 1/2026/A")
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>(),
            vec![
                EntityKind::Money,
                EntityKind::Date,
                EntityKind::Duration,
                EntityKind::Percent,
                EntityKind::Reference,
            ]
        );
    }

    #[test]
    fn malformed_input_never_panics() {
        for text in [
            "",
            "....",
            ",,,,",
            "1.2.3.4.5",
            "%%%%",
            "-",
            "₫₫₫",
            "999999999999999999999999₫",
        ] {
            let _ = extract_entities(text);
        }
    }
}
