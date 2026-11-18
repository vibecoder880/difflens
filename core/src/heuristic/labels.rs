//! Exported i18n label keys for every heuristic rule the engine can emit.
//!
//! Phase 7's frontend translation files validate against
//! [`ALL_LABEL_KEYS`] — adding a rule here without a key (or emitting an
//! unlisted key) breaks that contract, so keep this list exhaustive.
//! Keys follow the `flag.<camelCase>` convention documented in
//! `core/src/model.rs`.

use super::deltas;

/// All label keys the heuristic layer may emit, sorted alphabetically
/// (deterministic order for snapshotting).
pub const ALL_LABEL_KEYS: &[&str] = &[
    "flag.clauseRenamed",
    "flag.dateChanged",
    "flag.durationChanged",
    "flag.entityAdded",
    "flag.entityRemoved",
    "flag.keywordRisk",
    "flag.moneyChanged",
    "flag.percentChanged",
    "flag.referenceChanged",
];

/// Map a rule id to its i18n label key (single source of truth).
pub fn label_for(rule_id: &str) -> &'static str {
    match rule_id {
        deltas::RULE_MONEY_CHANGED => "flag.moneyChanged",
        deltas::RULE_DATE_CHANGED => "flag.dateChanged",
        deltas::RULE_DURATION_CHANGED => "flag.durationChanged",
        deltas::RULE_PERCENT_CHANGED => "flag.percentChanged",
        deltas::RULE_ENTITY_ADDED => "flag.entityAdded",
        deltas::RULE_ENTITY_REMOVED => "flag.entityRemoved",
        deltas::RULE_CLAUSE_RENAMED => "flag.clauseRenamed",
        deltas::RULE_REFERENCE_CHANGED => "flag.referenceChanged",
        deltas::RULE_KEYWORD_RISK => "flag.keywordRisk",
        _ => "flag.unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_id_has_a_label_key() {
        for rule_id in [
            deltas::RULE_MONEY_CHANGED,
            deltas::RULE_DATE_CHANGED,
            deltas::RULE_DURATION_CHANGED,
            deltas::RULE_PERCENT_CHANGED,
            deltas::RULE_ENTITY_ADDED,
            deltas::RULE_ENTITY_REMOVED,
            deltas::RULE_CLAUSE_RENAMED,
            deltas::RULE_REFERENCE_CHANGED,
            deltas::RULE_KEYWORD_RISK,
        ] {
            let key = label_for(rule_id);
            assert!(
                ALL_LABEL_KEYS.contains(&key),
                "rule {rule_id} label {key} missing from ALL_LABEL_KEYS"
            );
            assert!(key.starts_with("flag."), "key {key} breaks convention");
            // camelCase after the dot: first letter lowercase, no _ or -.
            let body = &key["flag.".len()..];
            assert!(
                body.chars()
                    .next()
                    .map(|c| c.is_lowercase())
                    .unwrap_or(false)
                    && !body.contains('_')
                    && !body.contains('-'),
                "key {key} is not camelCase"
            );
        }
    }

    #[test]
    fn label_keys_list_is_sorted_and_unique() {
        assert!(ALL_LABEL_KEYS.windows(2).all(|w| w[0] < w[1]));
    }
}
