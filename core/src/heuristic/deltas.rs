//! Old→new entity pairing and delta computation.
//!
//! For a matched pair of texts (Modified/Moved changes), entities are
//! grouped by kind and paired greedily by **closest canonical value**
//! (deterministic tie-break: lowest old span, then lowest new span). Pairs
//! whose canonical values are equal fire nothing; pairs that differ fire
//! one rule each:
//!
//! - [`RULE_MONEY_CHANGED`] — relative + absolute delta, currency-aware
//! - [`RULE_DATE_CHANGED`] — day difference
//! - [`RULE_DURATION_CHANGED`] — relative delta on day-equivalents
//! - [`RULE_PERCENT_CHANGED`] — relative delta + percentage points
//! - [`RULE_REFERENCE_CHANGED`] — normalized id changed
//!
//! Unpaired leftovers become [`RULE_ENTITY_REMOVED`] / [`RULE_ENTITY_ADDED`].

use crate::model::HeuristicFlag;

use super::entities::{Entity, EntityKind, EntityValue};
use super::labels;

pub const RULE_MONEY_CHANGED: &str = "MONEY_CHANGED";
pub const RULE_DATE_CHANGED: &str = "DATE_CHANGED";
pub const RULE_DURATION_CHANGED: &str = "DURATION_CHANGED";
pub const RULE_PERCENT_CHANGED: &str = "PERCENT_CHANGED";
pub const RULE_ENTITY_ADDED: &str = "ENTITY_ADDED";
pub const RULE_ENTITY_REMOVED: &str = "ENTITY_REMOVED";
pub const RULE_CLAUSE_RENAMED: &str = "CLAUSE_RENAMED";
pub const RULE_REFERENCE_CHANGED: &str = "REFERENCE_CHANGED";
pub const RULE_KEYWORD_RISK: &str = "KEYWORD_RISK";

/// Epsilon for comparing canonical float values (values come from the same
/// parser, so equal formatting is bit-equal; the epsilon only guards
/// accumulated scale arithmetic).
const VALUE_EPS: f64 = 1e-9;

/// Pair old/new entities of one kind greedily by closest value.
///
/// Deterministic: candidates are ordered by `(quantized value distance,
/// old_span, new_span, old_index, new_index)`, so the same inputs always
/// resolve to the same pairs. Returns `(paired, leftover_old_indices,
/// leftover_new_indices)`.
fn pair_by_value(
    old: &[&Entity],
    new: &[&Entity],
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Candidate {
        dist_key: i64,
        old_span: (usize, usize),
        new_span: (usize, usize),
        old_idx: usize,
        new_idx: usize,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (i, oe) in old.iter().enumerate() {
        for (j, ne) in new.iter().enumerate() {
            let d = (oe.value.numeric_key() - ne.value.numeric_key()).abs();
            // Quantized distance keeps the ordering key total (`i64`) and
            // therefore deterministic across runs.
            let dist_key = (d * 1e6).round() as i64;
            candidates.push(Candidate {
                dist_key,
                old_span: oe.span,
                new_span: ne.span,
                old_idx: i,
                new_idx: j,
            });
        }
    }
    candidates.sort();
    let mut used_old = vec![false; old.len()];
    let mut used_new = vec![false; new.len()];
    let mut paired = Vec::new();
    for c in candidates {
        if used_old[c.old_idx] || used_new[c.new_idx] {
            continue;
        }
        used_old[c.old_idx] = true;
        used_new[c.new_idx] = true;
        paired.push((c.old_idx, c.new_idx));
    }
    let leftover_old = (0..old.len()).filter(|i| !used_old[*i]).collect();
    let leftover_new = (0..new.len()).filter(|j| !used_new[*j]).collect();
    (paired, leftover_old, leftover_new)
}

/// Pair references by normalized string equality (exact id match wins);
/// everything else is added/removed.
fn pair_references(
    old: &[&Entity],
    new: &[&Entity],
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let mut used_new = vec![false; new.len()];
    let mut old_used = vec![false; old.len()];
    let mut paired = Vec::new();
    for (i, oe) in old.iter().enumerate() {
        let EntityValue::Reference { normalized: a } = &oe.value else {
            continue;
        };
        for (j, ne) in new.iter().enumerate() {
            if used_new[j] {
                continue;
            }
            if let EntityValue::Reference { normalized: b } = &ne.value
                && a == b
            {
                old_used[i] = true;
                used_new[j] = true;
                paired.push((i, j));
                break;
            }
        }
    }
    // Positional pairing of the remaining (edited) ids so an edited
    // reference fires RULE_REFERENCE_CHANGED instead of added/removed.
    let mut rem_old: Vec<usize> = (0..old.len()).filter(|i| !old_used[*i]).collect();
    let mut rem_new: Vec<usize> = (0..new.len()).filter(|j| !used_new[*j]).collect();
    while let (Some(i), Some(j)) = (rem_old.pop(), rem_new.pop()) {
        old_used[i] = true;
        used_new[j] = true;
        paired.push((i, j));
    }
    let leftover_old = (0..old.len()).filter(|i| !old_used[*i]).collect();
    let leftover_new = (0..new.len()).filter(|j| !used_new[*j]).collect();
    (paired, leftover_old, leftover_new)
}

/// Build the flag for a changed pair; `None` when canonical values match.
fn pair_flag(old_e: &Entity, new_e: &Entity) -> Option<HeuristicFlag> {
    let rule_id = match old_e.kind {
        EntityKind::Money => RULE_MONEY_CHANGED,
        EntityKind::Date => RULE_DATE_CHANGED,
        EntityKind::Duration => RULE_DURATION_CHANGED,
        EntityKind::Percent => RULE_PERCENT_CHANGED,
        EntityKind::Reference => RULE_REFERENCE_CHANGED,
    };
    let old_v = &old_e.value;
    let new_v = &new_e.value;
    if old_v.same_value(new_v) {
        return None;
    }
    let (delta, detail) = match (old_v, new_v) {
        (
            EntityValue::Money {
                amount: a,
                currency: ca,
                scale_note: sa,
            },
            EntityValue::Money {
                amount: b,
                currency: cb,
                scale_note: sb,
            },
        ) => {
            let rel = if a.abs() > VALUE_EPS {
                Some((b - a) / a.abs())
            } else {
                None
            };
            let mut d = format!(
                "{}→{} (abs {}, rel {:.4})",
                fmt_money(*a, *ca),
                fmt_money(*b, *cb),
                fmt_money(b - a, *cb),
                rel.unwrap_or(0.0)
            );
            if ca != cb {
                d.push_str(&format!("; currency changed {}→{}", ca.code(), cb.code()));
            }
            match (sa, sb) {
                (Some(x), Some(y)) if x != y => d.push_str(&format!("; scale {x}→{y}")),
                (Some(x), None) => d.push_str(&format!("; scale {x}→exact")),
                (None, Some(y)) => d.push_str(&format!("; scale exact→{y}")),
                _ => {}
            }
            (rel, d)
        }
        (
            EntityValue::Date {
                iso: a,
                day_number: da,
            },
            EntityValue::Date {
                iso: b,
                day_number: db,
            },
        ) => (
            Some((*db - *da) as f64),
            format!("{a}→{b} ({} days)", db - da),
        ),
        (
            EntityValue::Duration {
                days_equivalent: a,
                unit: ua,
            },
            EntityValue::Duration {
                days_equivalent: b,
                unit: ub,
            },
        ) => {
            let rel = if a.abs() > VALUE_EPS {
                Some((b - a) / a.abs())
            } else {
                None
            };
            let d = format!(
                "{}→{} day-equivalents (unit {}→{})",
                fmt_f64(*a),
                fmt_f64(*b),
                ua,
                ub
            );
            (rel, d)
        }
        (EntityValue::Percent { value: a }, EntityValue::Percent { value: b }) => {
            let rel = if a.abs() > VALUE_EPS {
                Some((b - a) / a.abs())
            } else {
                None
            };
            let pp = b - a;
            let d = format!("{:.2}%→{:.2}% ({pp:+.2} pp)", a, b);
            (rel, d)
        }
        (EntityValue::Reference { normalized: a }, EntityValue::Reference { normalized: b }) => {
            (None, format!("{a}→{b}"))
        }
        _ => {
            // Kinds are grouped before pairing; mixed kinds never occur.
            (None, format!("{}→{}", old_e.text, new_e.text))
        }
    };
    Some(HeuristicFlag {
        rule_id: rule_id.to_string(),
        label_key: labels::label_for(rule_id).to_string(),
        old_value: Some(old_e.text.clone()),
        new_value: Some(new_e.text.clone()),
        delta,
        detail: Some(detail),
    })
}

/// Build an added/removed flag for an unpaired entity. Public because the
/// orchestrator also uses it for pure Added/Removed changes (no
/// counterpart to pair against).
pub fn side_flag(e: &Entity, added: bool) -> HeuristicFlag {
    let rule_id = if added {
        RULE_ENTITY_ADDED
    } else {
        RULE_ENTITY_REMOVED
    };
    let describe = match &e.value {
        EntityValue::Money {
            amount,
            currency,
            scale_note,
        } => format!(
            "{} {}{}",
            fmt_f64(*amount),
            currency.code(),
            scale_note
                .map(|s| format!(" (scale {s})"))
                .unwrap_or_default()
        ),
        EntityValue::Date { iso, .. } => iso.clone(),
        EntityValue::Duration {
            days_equivalent,
            unit,
        } => format!("{} ({unit}) day-equivalents", fmt_f64(*days_equivalent)),
        EntityValue::Percent { value } => format!("{value}%"),
        EntityValue::Reference { normalized } => normalized.clone(),
    };
    HeuristicFlag {
        rule_id: rule_id.to_string(),
        label_key: labels::label_for(rule_id).to_string(),
        old_value: if added { None } else { Some(e.text.clone()) },
        new_value: if added { Some(e.text.clone()) } else { None },
        delta: None,
        detail: Some(format!("{}: {describe}", e.kind.name())),
    }
}

/// Format a money amount compactly (deterministic, no locale dependence).
fn fmt_money(v: f64, c: super::entities::Currency) -> String {
    format!("{} {}", fmt_f64(v), c.code())
}

/// Fixed-format float: integers plain, else up to 2 decimals with trailing
/// zeros trimmed. Avoids platform-dependent `{:?}` output.
fn fmt_f64(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Compute all delta flags for one old/new text pair.
///
/// Flags come out in deterministic order: paired deltas grouped by kind in
/// [`EntityKind`] order (pairs ordered by old span), then removed
/// leftovers, then added leftovers (each ordered by span).
pub fn compute_deltas(old_text: &str, new_text: &str) -> Vec<HeuristicFlag> {
    let old_ents = super::entities::extract_entities(old_text);
    let new_ents = super::entities::extract_entities(new_text);
    compute_deltas_for(&old_ents, &new_ents)
}

/// [`compute_deltas`] over pre-extracted entities (the orchestrator reuses
/// extractions to avoid scanning texts twice).
pub fn compute_deltas_for(old_ents: &[Entity], new_ents: &[Entity]) -> Vec<HeuristicFlag> {
    let mut flags = Vec::new();
    for kind in [
        EntityKind::Money,
        EntityKind::Date,
        EntityKind::Duration,
        EntityKind::Percent,
        EntityKind::Reference,
    ] {
        let old_group: Vec<&Entity> = old_ents.iter().filter(|e| e.kind == kind).collect();
        let new_group: Vec<&Entity> = new_ents.iter().filter(|e| e.kind == kind).collect();
        let (mut paired, leftover_old, leftover_new) = if kind == EntityKind::Reference {
            pair_references(&old_group, &new_group)
        } else {
            pair_by_value(&old_group, &new_group)
        };
        // Emit pairs in old-span order for stable audit trails.
        paired.sort_by(|a, b| {
            old_group[a.0]
                .span
                .cmp(&old_group[b.0].span)
                .then_with(|| new_group[a.1].span.cmp(&new_group[b.1].span))
        });
        for (i, j) in paired {
            if let Some(f) = pair_flag(old_group[i], new_group[j]) {
                flags.push(f);
            }
        }
        for i in leftover_old {
            flags.push(side_flag(old_group[i], false));
        }
        for j in leftover_new {
            flags.push(side_flag(new_group[j], true));
        }
    }
    flags
}

// ---------------------------------------------------------------------------
// Tests: pairing + delta math
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(old: &str, new: &str) -> Vec<HeuristicFlag> {
        compute_deltas(old, new)
    }

    fn rule_of(f: &HeuristicFlag) -> &str {
        &f.rule_id
    }

    #[test]
    fn duration_halving_is_minus_half() {
        let fs = flags("payment within 30 days", "payment within 15 days");
        assert_eq!(fs.len(), 1);
        assert_eq!(rule_of(&fs[0]), RULE_DURATION_CHANGED);
        assert!((fs[0].delta.unwrap() - (-0.5)).abs() < 1e-9);
        assert_eq!(fs[0].old_value.as_deref(), Some("within 30 days"));
        assert_eq!(fs[0].new_value.as_deref(), Some("within 15 days"));
    }

    #[test]
    fn money_vn_1500k_to_2ty_relative_delta() {
        // (2e9 − 1.5e6) / 1.5e6 = 1332.3333… (i.e. a ×1333.33 total).
        let fs = flags("Phí là 1.500.000₫ mỗi tháng.", "Phí là 2 tỷ mỗi tháng.");
        assert_eq!(fs.len(), 1);
        assert_eq!(rule_of(&fs[0]), RULE_MONEY_CHANGED);
        let d = fs[0].delta.unwrap();
        assert!((d - 1_332.333_333).abs() < 1e-4, "delta {d}");
        let detail = fs[0].detail.as_deref().unwrap().to_string();
        assert!(detail.contains("VND"), "detail: {detail}");
    }

    #[test]
    fn date_delta_is_day_difference() {
        let fs = flags("hạn 15/03/2026", "hạn 15/03/2027");
        assert_eq!(fs.len(), 1);
        assert_eq!(rule_of(&fs[0]), RULE_DATE_CHANGED);
        assert_eq!(fs[0].delta.unwrap(), 365.0);
        assert!(
            fs[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("2026-03-15→2027-03-15")
        );
    }

    #[test]
    fn percent_delta_has_percentage_points_in_detail() {
        let fs = flags("chiết khấu 5%", "chiết khấu 12,5%");
        assert_eq!(rule_of(&fs[0]), RULE_PERCENT_CHANGED);
        assert!((fs[0].delta.unwrap() - 1.5).abs() < 1e-9);
        assert!(fs[0].detail.as_deref().unwrap().contains("+7.50 pp"));
    }

    #[test]
    fn equal_values_fire_nothing() {
        // Same canonical value written differently must not flag.
        assert!(flags("phí 1.500.000₫", "phí 1500000 VND").is_empty());
        assert!(flags("hạn 15/03/2026", "hạn ngày 15 tháng 3 năm 2026").is_empty());
    }

    #[test]
    fn closest_value_pairing_with_leftover() {
        // Old has 1.5m and 500m; new has only 2e9 → the 500m pairs (closest
        // value), the 1.5m becomes ENTITY_REMOVED.
        let fs = flags("cọc 1.500.000₫ và tổng 500 triệu đồng", "tổng 2 tỷ");
        let rules: Vec<&str> = fs.iter().map(rule_of).collect();
        assert!(rules.contains(&RULE_MONEY_CHANGED));
        assert!(rules.contains(&RULE_ENTITY_REMOVED));
        let changed = fs
            .iter()
            .find(|f| f.rule_id == RULE_MONEY_CHANGED)
            .expect("money pair fired");
        assert!(changed.old_value.as_deref().unwrap().contains("500 triệu"));
    }

    #[test]
    fn entity_added_flag() {
        let fs = flags("thuần túy văn bản", "văn bản mới thêm 5% phí");
        assert_eq!(fs.len(), 1);
        assert_eq!(rule_of(&fs[0]), RULE_ENTITY_ADDED);
        assert_eq!(fs[0].old_value, None);
        assert_eq!(fs[0].new_value.as_deref(), Some("5%"));
        assert_eq!(fs[0].detail.as_deref(), Some("percent: 5%"));
    }

    #[test]
    fn entity_removed_flag() {
        let fs = flags("tiền phạt 5.000.000₫", "không còn mức phạt");
        assert_eq!(fs.len(), 1);
        assert_eq!(rule_of(&fs[0]), RULE_ENTITY_REMOVED);
        assert_eq!(fs[0].new_value, None);
    }

    #[test]
    fn reference_changed_and_matching() {
        let fs = flags("số: 123/2026/HD", "số: 124/2026/HD");
        assert_eq!(fs.len(), 1);
        assert_eq!(rule_of(&fs[0]), RULE_REFERENCE_CHANGED);
        assert!(fs[0].delta.is_none());
        // Identical references cancel out.
        assert!(flags("số: 123/2026/HD", "số 123/2026/HD").is_empty());
    }

    #[test]
    fn currency_mismatch_noted_in_detail() {
        let fs = flags("phạt $100", "phạt $200");
        assert_eq!(fs.len(), 1);
        assert!((fs[0].delta.unwrap() - 1.0).abs() < 1e-9);
        let fs = flags("giá 100.000₫", "giá $100");
        assert!(fs.iter().any(|f| {
            f.detail
                .as_deref()
                .unwrap_or_default()
                .contains("currency changed VND→USD")
        }));
    }

    #[test]
    fn mixed_kinds_fire_each_one_rule() {
        let fs = flags(
            "trả 1.000.000₫ trước 15/01/2026, lãi 5%",
            "trả 2.000.000₫ trước 15/01/2026, lãi 6%",
        );
        let rules: Vec<&str> = fs.iter().map(rule_of).collect();
        assert!(rules.contains(&RULE_MONEY_CHANGED));
        assert!(rules.contains(&RULE_PERCENT_CHANGED));
        assert!(!rules.contains(&RULE_DATE_CHANGED), "equal date cancels");
    }

    #[test]
    fn empty_inputs_no_flags() {
        assert!(flags("", "").is_empty());
        assert!(flags("abc", "xyz").is_empty());
    }
}
