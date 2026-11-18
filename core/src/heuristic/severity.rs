//! Deterministic banded severity scoring.
//!
//! ```text
//! score = min(100, max(entity_rule_points) + keyword_uplift)
//! ```
//!
//! - `entity_rule_points` — points contributed by each fired entity rule
//!   (see [`entity_rule_points`]); the change takes the **max**, not the
//!   sum, so a change with many flags scores on its strongest evidence.
//! - `keyword_uplift` — the highest-weight risk lexicon category present in
//!   the relevant text (changed inline segments for Modified/Moved pairs,
//!   the whole block for Added/Removed), plus a small breadth bonus when
//!   ≥ 3 distinct categories match.
//!
//! Bands ([`band`]): score 0 → Info, 1–29 Low, 30–59 Medium, 60–79 High,
//! ≥ 80 Critical (the phase-plan thresholds). Pure formatting changes
//! (only whitespace/punctuation/case differ) score 0 → Info.

use regex::Regex;
use std::sync::OnceLock;

use crate::model::{HeuristicFlag, Severity};

use super::deltas;
use super::entities::EntityKind;
use super::labels;

/// Band thresholds.
pub const BAND_CRITICAL_MIN: u32 = 80;
pub const BAND_HIGH_MIN: u32 = 60;
pub const BAND_MEDIUM_MIN: u32 = 30;

/// Base points per entity rule.
pub const POINTS_MONEY: u32 = 35;
pub const POINTS_DATE: u32 = 30;
pub const POINTS_DURATION: u32 = 30;
pub const POINTS_PERCENT: u32 = 30;
pub const POINTS_REFERENCE: u32 = 20;
pub const POINTS_CLAUSE_RENAMED: u32 = 15;
pub const POINTS_ENTITY_SIDE_RICH: u32 = 15; // added/removed money or date
pub const POINTS_ENTITY_SIDE_PLAIN: u32 = 10; // other added/removed entities

/// Keyword breadth bonus (≥ 3 distinct categories matched).
pub const KEYWORD_BREADTH_BONUS: u32 = 5;
/// Minimum distinct categories for the breadth bonus.
pub const KEYWORD_BREADTH_MIN: u32 = 3;

/// A risk-lexicon entry: one category, its weight, and the VN+EN surface
/// forms that trigger it (case-insensitive, NFC-normalized text).
struct LexiconEntry {
    category: &'static str,
    weight: u32,
    pattern: &'static str,
}

const LEXICON: &[LexiconEntry] = &[
    // Highest band contributors: termination / liability / indemnity class.
    LexiconEntry {
        category: "termination",
        weight: 25,
        pattern: r"termination|chấm dứt",
    },
    LexiconEntry {
        category: "liability",
        weight: 25,
        pattern: r"liability|miễn trừ trách nhiệm|giới hạn trách nhiệm",
    },
    LexiconEntry {
        category: "indemnity",
        weight: 25,
        pattern: r"indemnity|indemnify|bồi thường",
    },
    LexiconEntry {
        category: "penalty",
        weight: 20,
        pattern: r"penalt(?:y|ies)|phạt",
    },
    LexiconEntry {
        category: "collateral",
        weight: 20,
        pattern: r"collateral|tài sản bảo đảm",
    },
    LexiconEntry {
        category: "warranty",
        weight: 15,
        pattern: r"warrant(?:y|ies)|bảo hành",
    },
    LexiconEntry {
        category: "confidentiality",
        weight: 15,
        pattern: r"confidential|bảo mật",
    },
    LexiconEntry {
        category: "intellectual_property",
        weight: 15,
        pattern: r"intellectual\s+property|tài sản trí tuệ",
    },
    LexiconEntry {
        category: "assignment",
        weight: 15,
        pattern: r"assignment|chuyển nhượng",
    },
    LexiconEntry {
        category: "interest_rate",
        weight: 15,
        pattern: r"interest\s+rate|lãi suất",
    },
    LexiconEntry {
        category: "governing_law",
        weight: 10,
        pattern: r"governing\s+law|luật áp dụng",
    },
    LexiconEntry {
        category: "dispute",
        weight: 10,
        pattern: r"disputes?|tranh chấp",
    },
    LexiconEntry {
        category: "payment",
        weight: 10,
        pattern: r"payments?|thanh toán",
    },
    LexiconEntry {
        category: "validity",
        weight: 10,
        pattern: r"hiệu lực|validity",
    },
    LexiconEntry {
        category: "deadline",
        weight: 10,
        pattern: r"deadline|hạn chót|due date",
    },
];

fn lexicon_regexes() -> &'static [(LexiconEntryProxy, Regex)] {
    static RE: OnceLock<Vec<(LexiconEntryProxy, Regex)>> = OnceLock::new();
    RE.get_or_init(|| {
        LEXICON
            .iter()
            .map(|e| {
                (
                    LexiconEntryProxy {
                        category: e.category,
                        weight: e.weight,
                    },
                    Regex::new(&format!("(?i){}", e.pattern)).expect("lexicon regex"),
                )
            })
            .collect()
    })
}

/// 'static slice of the entry metadata (regexes live next to them).
#[derive(Debug)]
struct LexiconEntryProxy {
    category: &'static str,
    weight: u32,
}

/// Keyword uplift over `text`: `(max_weight, matched_categories_sorted)`.
pub fn keyword_uplift(text: &str) -> (u32, Vec<&'static str>) {
    let mut max_weight = 0u32;
    let mut categories = Vec::new();
    for (entry, re) in lexicon_regexes() {
        if re.is_match(text) {
            categories.push(entry.category);
            max_weight = max_weight.max(entry.weight);
        }
    }
    if categories.len() as u32 >= KEYWORD_BREADTH_MIN {
        max_weight += KEYWORD_BREADTH_BONUS;
    }
    categories.sort_unstable();
    (max_weight, categories)
}

/// Points contributed by a single fired entity flag.
pub fn entity_rule_points(flag: &HeuristicFlag) -> u32 {
    match flag.rule_id.as_str() {
        deltas::RULE_MONEY_CHANGED => {
            let rel = flag.delta.unwrap_or(0.0).abs();
            let mut p = POINTS_MONEY;
            if rel >= 0.10 {
                p += 10;
            }
            if rel >= 0.50 {
                p += 10;
            }
            if rel >= 1.00 {
                p += 10;
            }
            p
        }
        deltas::RULE_DATE_CHANGED => {
            let days = flag.delta.unwrap_or(0.0).abs();
            let tier = if days >= 365.0 {
                15
            } else if days >= 90.0 {
                10
            } else if days >= 30.0 {
                5
            } else {
                0
            };
            POINTS_DATE + tier
        }
        deltas::RULE_DURATION_CHANGED => {
            let rel = flag.delta.unwrap_or(0.0);
            let tier = if rel.abs() >= 0.50 {
                15
            } else if rel.abs() >= 0.25 {
                10
            } else {
                0
            };
            // Shortening an obligation is riskier than extending it.
            let shortened = if rel < 0.0 { 5 } else { 0 };
            POINTS_DURATION + tier + shortened
        }
        deltas::RULE_PERCENT_CHANGED => {
            // Relative delta is stored; approximate percentage points from
            // old/new evidence when both are present, else from the ratio.
            let pp = percent_points(flag);
            let tier = if pp >= 10.0 {
                15
            } else if pp >= 5.0 {
                10
            } else if pp >= 1.0 {
                5
            } else {
                0
            };
            POINTS_PERCENT + tier
        }
        deltas::RULE_REFERENCE_CHANGED => POINTS_REFERENCE,
        deltas::RULE_CLAUSE_RENAMED => POINTS_CLAUSE_RENAMED,
        deltas::RULE_ENTITY_ADDED | deltas::RULE_ENTITY_REMOVED => {
            match entity_kind_of_flag(flag) {
                Some(EntityKind::Money) | Some(EntityKind::Date) => POINTS_ENTITY_SIDE_RICH,
                _ => POINTS_ENTITY_SIDE_PLAIN,
            }
        }
        _ => 0,
    }
}

/// Percentage-point magnitude for a percent-change flag: parses the two
/// `x.xx%` leads from `detail` (`"5.00%→12.50% (+7.50 pp)"`), falling back
/// to `|delta|` when the detail shape is unexpected.
fn percent_points(flag: &HeuristicFlag) -> f64 {
    if let Some(detail) = &flag.detail
        && let Some(arrow) = detail.find('→')
    {
        let old = detail[..arrow].trim_end_matches('%').trim_end();
        let after = &detail[arrow + '→'.len_utf8()..];
        let new = after.split('%').next().unwrap_or("").trim_start();
        if let (Ok(o), Ok(n)) = (old.parse::<f64>(), new.parse::<f64>()) {
            return (n - o).abs();
        }
    }
    flag.delta.unwrap_or(0.0).abs()
}

/// Entity kind for a flag: paired rules imply the kind from the rule id;
/// added/removed flags carry it in the `detail` prefix (`"money: …"`).
fn entity_kind_of_flag(flag: &HeuristicFlag) -> Option<EntityKind> {
    match flag.rule_id.as_str() {
        deltas::RULE_MONEY_CHANGED => Some(EntityKind::Money),
        deltas::RULE_DATE_CHANGED => Some(EntityKind::Date),
        deltas::RULE_DURATION_CHANGED => Some(EntityKind::Duration),
        deltas::RULE_PERCENT_CHANGED => Some(EntityKind::Percent),
        deltas::RULE_REFERENCE_CHANGED => Some(EntityKind::Reference),
        deltas::RULE_ENTITY_ADDED | deltas::RULE_ENTITY_REMOVED => {
            let detail = flag.detail.as_deref()?;
            let kind = detail.split(':').next()?;
            match kind.trim() {
                "money" => Some(EntityKind::Money),
                "date" => Some(EntityKind::Date),
                "duration" => Some(EntityKind::Duration),
                "percent" => Some(EntityKind::Percent),
                "reference" => Some(EntityKind::Reference),
                _ => None,
            }
        }
        _ => None,
    }
}

/// True when two texts differ only by whitespace, punctuation, or case.
pub fn is_formatting_only(old: &str, new: &str) -> bool {
    let squash = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect::<String>()
    };
    squash(old) == squash(new)
}

/// Band for a 0..=100 score.
pub fn band(score: u32) -> Severity {
    match score {
        0 => Severity::Info,
        s if s >= BAND_CRITICAL_MIN => Severity::Critical,
        s if s >= BAND_HIGH_MIN => Severity::High,
        s if s >= BAND_MEDIUM_MIN => Severity::Medium,
        _ => Severity::Low,
    }
}

/// Result of scoring one change.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub score: u32,
    pub severity: Severity,
    /// `KEYWORD_RISK` flag when the lexicon fired (appended to the audit
    /// trail so every severity point is explainable).
    pub keyword_flag: Option<HeuristicFlag>,
}

/// Score one change from its entity flags and the relevant text.
///
/// `formatting_only` short-circuits to Info (the caller computes it from
/// the old/new block texts; only Modified changes pass `true`).
pub fn score(relevant_text: &str, entity_flags: &[HeuristicFlag], formatting_only: bool) -> Scored {
    if formatting_only {
        return Scored {
            score: 0,
            severity: Severity::Info,
            keyword_flag: None,
        };
    }
    let entity_max = entity_flags
        .iter()
        .map(entity_rule_points)
        .max()
        .unwrap_or(0);
    let (keyword_weight, categories) = keyword_uplift(relevant_text);
    let keyword_flag = (!categories.is_empty()).then(|| HeuristicFlag {
        rule_id: deltas::RULE_KEYWORD_RISK.to_string(),
        label_key: labels::label_for(deltas::RULE_KEYWORD_RISK).to_string(),
        old_value: None,
        new_value: None,
        delta: None,
        detail: Some(categories.join(", ")),
    });
    let score = (entity_max + keyword_weight).min(100);
    Scored {
        score,
        severity: band(score),
        keyword_flag,
    }
}

// ---------------------------------------------------------------------------
// Tests: banding table, lexicon, rule points
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(rule_id: &str, delta: Option<f64>, detail: Option<&str>) -> HeuristicFlag {
        HeuristicFlag {
            rule_id: rule_id.to_string(),
            label_key: labels::label_for(rule_id).to_string(),
            old_value: None,
            new_value: None,
            delta,
            detail: detail.map(str::to_string),
        }
    }

    #[test]
    fn band_thresholds_pinned() {
        assert_eq!(band(0), Severity::Info);
        assert_eq!(band(1), Severity::Low);
        assert_eq!(band(29), Severity::Low);
        assert_eq!(band(30), Severity::Medium);
        assert_eq!(band(59), Severity::Medium);
        assert_eq!(band(60), Severity::High);
        assert_eq!(band(79), Severity::High);
        assert_eq!(band(80), Severity::Critical);
        assert_eq!(band(100), Severity::Critical);
    }

    #[test]
    fn money_points_by_magnitude() {
        // Base 35: rel 0.05 → no tier.
        assert_eq!(
            entity_rule_points(&flag("MONEY_CHANGED", Some(0.05), None)),
            35
        );
        // 1.500.000₫ → 2 tỷ: rel ≈ 1332 → all tiers → 65.
        assert_eq!(
            entity_rule_points(&flag("MONEY_CHANGED", Some(1332.33), None)),
            65
        );
        assert_eq!(
            entity_rule_points(&flag("MONEY_CHANGED", Some(0.6), None)),
            55
        );
        assert_eq!(
            entity_rule_points(&flag("MONEY_CHANGED", Some(0.2), None)),
            45
        );
    }

    #[test]
    fn duration_points_shortened_vs_extended() {
        // 30 → 15 days: rel −0.5 → tier 15 + shortened 5 → 50.
        assert_eq!(
            entity_rule_points(&flag("DURATION_CHANGED", Some(-0.5), None)),
            50
        );
        // 15 → 30 days: same tier, no shortened bonus → 45.
        assert_eq!(
            entity_rule_points(&flag("DURATION_CHANGED", Some(1.0), None)),
            45
        );
        // Small tweak → base only.
        assert_eq!(
            entity_rule_points(&flag("DURATION_CHANGED", Some(-0.1), None)),
            35
        );
    }

    #[test]
    fn date_points_by_day_distance() {
        assert_eq!(
            entity_rule_points(&flag("DATE_CHANGED", Some(5.0), None)),
            30
        );
        assert_eq!(
            entity_rule_points(&flag("DATE_CHANGED", Some(45.0), None)),
            35
        );
        assert_eq!(
            entity_rule_points(&flag("DATE_CHANGED", Some(120.0), None)),
            40
        );
        assert_eq!(
            entity_rule_points(&flag("DATE_CHANGED", Some(365.0), None)),
            45
        );
    }

    #[test]
    fn percent_points_use_percentage_points() {
        assert_eq!(
            entity_rule_points(&flag(
                "PERCENT_CHANGED",
                Some(0.04),
                Some("5.00%→5.20% (+0.20 pp)")
            )),
            30
        );
        assert_eq!(
            entity_rule_points(&flag(
                "PERCENT_CHANGED",
                Some(1.5),
                Some("5.00%→12.50% (+7.50 pp)")
            )),
            40
        );
        assert_eq!(
            entity_rule_points(&flag(
                "PERCENT_CHANGED",
                Some(9.0),
                Some("5.00%→50.00% (+45.00 pp)")
            )),
            45
        );
    }

    #[test]
    fn clause_rename_and_reference_points() {
        assert_eq!(entity_rule_points(&flag("CLAUSE_RENAMED", None, None)), 15);
        assert_eq!(
            entity_rule_points(&flag("REFERENCE_CHANGED", None, None)),
            20
        );
    }

    #[test]
    fn added_removed_points_by_entity_kind() {
        assert_eq!(
            entity_rule_points(&flag("ENTITY_REMOVED", None, Some("money: 100000 VND"))),
            15
        );
        assert_eq!(
            entity_rule_points(&flag("ENTITY_ADDED", None, Some("date: 2026-03-15"))),
            15
        );
        assert_eq!(
            entity_rule_points(&flag("ENTITY_ADDED", None, Some("percent: 5%"))),
            10
        );
    }

    #[test]
    fn lexicon_weights_vn_and_en() {
        assert_eq!(keyword_uplift("điều khoản chấm dứt hợp đồng").0, 25);
        assert_eq!(keyword_uplift("In case of termination").0, 25);
        assert_eq!(keyword_uplift("bồi thường thiệt hại").0, 25);
        assert_eq!(keyword_uplift("mức phạt vi phạm").0, 20);
        assert_eq!(keyword_uplift("thanh toán từng đợt").0, 10);
        assert_eq!(keyword_uplift("văn bản vô hại").0, 0);
        assert!(keyword_uplift("văn bản vô hại").1.is_empty());
    }

    #[test]
    fn lexicon_breadth_bonus_at_three_categories() {
        let (w, cats) = keyword_uplift("thanh toán, tranh chấp, bảo mật");
        assert_eq!(cats.len(), 3);
        // Max weight 15 (confidentiality) + breadth bonus 5.
        assert_eq!(w, 20);
    }

    #[test]
    fn keyword_flag_lists_categories_sorted() {
        let scored = score("bảo mật và chuyển nhượng và bảo hành", &[], false);
        let f = scored.keyword_flag.expect("keyword flag");
        assert_eq!(f.rule_id, "KEYWORD_RISK");
        assert_eq!(
            f.detail.as_deref(),
            Some("assignment, confidentiality, warranty")
        );
    }

    #[test]
    fn formatting_only_is_info() {
        assert!(is_formatting_only(
            "Hạn chót: 15/03/2026.",
            "Hạn chót  15/03/2026!"
        ));
        assert!(is_formatting_only("ABC", "abc"));
        assert!(!is_formatting_only("hạn 15/03/2026", "hạn 16/03/2026"));
        let scored = score("anything", &[], true);
        assert_eq!(scored.severity, Severity::Info);
        assert_eq!(scored.score, 0);
        assert!(scored.keyword_flag.is_none());
    }

    #[test]
    fn score_combines_max_entity_and_keywords_capped() {
        // Money change 65 + termination keyword 25 = 90 → Critical.
        let flags = [flag("MONEY_CHANGED", Some(1332.33), None)];
        let scored = score("điều khoản chấm dứt", &flags, false);
        assert_eq!(scored.score, 90);
        assert_eq!(scored.severity, Severity::Critical);
        // Broad keywords: max category 25 + breadth bonus 5 = 30; entity max
        // 65 + 30 = 95, the highest score the tables can produce (still
        // Critical). `score`'s `min(100, …)` is the safety clamp above it.
        let flags = [flag("MONEY_CHANGED", Some(1332.33), None)];
        let scored = score("chấm dứt, bồi thường, phạt", &flags, false);
        assert_eq!(scored.score, 95);
        assert_eq!(scored.severity, Severity::Critical);
        // The clamp itself: bands stay pinned even at the theoretical max.
        assert_eq!(band(100), Severity::Critical);
    }

    #[test]
    fn keywords_alone_score_added_removed() {
        let scored = score("Bên A có nghĩa vụ bồi thường", &[], false);
        assert_eq!(scored.score, 25);
        assert_eq!(scored.severity, Severity::Low);
        let scored = score("thanh toán đúng hạn", &[], false);
        assert_eq!(scored.score, 10);
        assert_eq!(scored.severity, Severity::Low);
        let scored = score("đoạn văn bình thường", &[], false);
        assert_eq!(scored.score, 0);
        assert_eq!(scored.severity, Severity::Info);
    }
}
