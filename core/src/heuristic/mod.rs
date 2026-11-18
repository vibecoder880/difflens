//! Heuristic intelligence layer: entity extraction, delta computation,
//! severity scoring, and the audit trail.
//!
//! Pipeline position: runs after diff/alignment, mutating every non-Equal
//! [`Change`] in place (fills `severity` + `flags`) and producing a
//! [`AuditEntry`] per touched change — `DiffResult.audit`.
//!
//! 100% deterministic: fixed rule/table order, `BTreeMap` lookups, no
//! HashMap iteration anywhere, no time or randomness. Same inputs always
//! yield byte-identical output (enforced by tests).

pub mod deltas;
pub mod entities;
pub mod labels;
pub mod severity;

pub use labels::ALL_LABEL_KEYS;

use std::collections::BTreeMap;

use crate::model::{
    AuditEntry, Block, Change, ChangeKind, DiffResult, HeuristicFlag, Severity, SpanOp,
};
// Submodules `deltas`, `entities`, `labels` are in scope by name as items
// of this module.

/// Enrich a whole `DiffResult` in place, attaching `severity`/`flags` to
/// every non-Equal change and returning the audit trail (also stored in
/// `result.audit`).
///
/// Block texts are resolved by id from the two document block lists (the
/// diff engine only stores ids on the changes, so enrichment needs the
/// sources for text/anchor lookups).
pub fn enrich(result: &mut DiffResult, old_blocks: &[Block], new_blocks: &[Block]) {
    let audit = enrich_changes(&mut result.changes, old_blocks, new_blocks);
    result.audit = audit;
}

/// Enrich a change list in place and return the audit trail (one
/// [`AuditEntry`] per enriched change, in change order — changes are
/// processed in list order, which is id order by construction).
pub fn enrich_changes(
    changes: &mut [Change],
    old_blocks: &[Block],
    new_blocks: &[Block],
) -> Vec<AuditEntry> {
    let old_by_id: BTreeMap<&str, &Block> = old_blocks.iter().map(|b| (b.id.as_str(), b)).collect();
    let new_by_id: BTreeMap<&str, &Block> = new_blocks.iter().map(|b| (b.id.as_str(), b)).collect();

    let mut audit = Vec::new();
    for change in changes.iter_mut() {
        // Equal changes carry no information; severity stays None.
        if change.kind == ChangeKind::Equal {
            continue;
        }
        let old_text = change
            .old_block_ids
            .first()
            .and_then(|id| old_by_id.get(id.as_str()))
            .map(|b| b.text.as_str())
            .unwrap_or_default();
        let new_text = change
            .new_block_ids
            .first()
            .and_then(|id| new_by_id.get(id.as_str()))
            .map(|b| b.text.as_str())
            .unwrap_or_default();

        // 1. Entity deltas for matched pairs (Modified/Moved) plus
        //    added/removed entity sides.
        let mut flags = if change.old_block_ids.is_empty() || change.new_block_ids.is_empty() {
            side_only_flags(old_text, new_text, change.kind)
        } else {
            deltas::compute_deltas(old_text, new_text)
        };

        // 2. Clause rename: both anchors present and different.
        let old_anchor = change
            .old_block_ids
            .first()
            .and_then(|id| old_by_id.get(id.as_str()))
            .and_then(|b| b.anchor.clone());
        let new_anchor = change
            .new_block_ids
            .first()
            .and_then(|id| new_by_id.get(id.as_str()))
            .and_then(|b| b.anchor.clone());
        if let (Some(a), Some(b)) = (&old_anchor, &new_anchor)
            && a != b
        {
            flags.push(HeuristicFlag {
                rule_id: deltas::RULE_CLAUSE_RENAMED.to_string(),
                label_key: labels::label_for(deltas::RULE_CLAUSE_RENAMED).to_string(),
                old_value: Some(a.clone()),
                new_value: Some(b.clone()),
                delta: None,
                detail: Some("anchor changed".to_string()),
            });
        }

        // 3. Relevant text for the lexicon: changed inline segments for
        //    Modified, the block text for Moved/Added/Removed. Formatting
        //    demotion applies to Modified only (a Moved change has
        //    identical text by construction).
        let (relevant, formatting_only) = match change.kind {
            ChangeKind::Modified => {
                let segments: String = change
                    .spans
                    .iter()
                    .filter(|s| s.op != SpanOp::Equal)
                    .map(|s| s.text.as_str())
                    .collect();
                (segments, severity::is_formatting_only(old_text, new_text))
            }
            ChangeKind::Moved => (old_text.to_string(), false),
            ChangeKind::Added => (new_text.to_string(), false),
            ChangeKind::Removed => (old_text.to_string(), false),
            ChangeKind::Equal => unreachable!("equal changes skipped above"),
        };

        // 4. Score, band, attach flags + audit entry.
        let scored = severity::score(&relevant, &flags, formatting_only);
        if let Some(kf) = scored.keyword_flag {
            flags.push(kf);
        }
        change.severity = Some(scored.severity);
        change.flags = flags.clone();
        audit.push(AuditEntry {
            change_id: change.id.clone(),
            severity: scored.severity,
            rules: flags,
        });
    }
    audit
}

/// Flags for pure Added/Removed changes: every entity on the present side
/// counts as added/removed (there is no counterpart to pair with).
fn side_only_flags(old_text: &str, new_text: &str, kind: ChangeKind) -> Vec<HeuristicFlag> {
    let (present, added) = match kind {
        ChangeKind::Added => (entities::extract_entities(new_text), true),
        ChangeKind::Removed => (entities::extract_entities(old_text), false),
        _ => (Vec::new(), true),
    };
    present
        .iter()
        .map(|e| deltas::side_flag(e, added))
        .collect()
}

/// Convenience wrapper used by the gold enrichment corpus: treat two texts
/// as a single Modified pair and return `(flags, severity)`.
pub fn enrich_texts(old_text: &str, new_text: &str) -> (Vec<HeuristicFlag>, Severity) {
    let spans = crate::diff::word_spans(old_text, new_text);
    let old_block = Block {
        id: "a0".to_string(),
        kind: crate::model::BlockKind::Paragraph,
        text: old_text.to_string(),
        page: None,
        rect: None,
        anchor: None,
    };
    let new_block = Block {
        id: "b0".to_string(),
        kind: crate::model::BlockKind::Paragraph,
        text: new_text.to_string(),
        page: None,
        rect: None,
        anchor: None,
    };
    let mut changes = vec![Change {
        id: "c0".to_string(),
        kind: ChangeKind::Modified,
        old_block_ids: vec!["a0".to_string()],
        new_block_ids: vec!["b0".to_string()],
        old_page: None,
        new_page: None,
        spans,
        similarity: None,
        move_delta: None,
        severity: None,
        flags: Vec::new(),
    }];
    let audit = enrich_changes(
        &mut changes,
        std::slice::from_ref(&old_block),
        std::slice::from_ref(&new_block),
    );
    let severity = audit.first().map(|a| a.severity).unwrap_or(Severity::Info);
    (
        changes.first().map(|c| c.flags.clone()).unwrap_or_default(),
        severity,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BlockKind;

    fn block(id: &str, text: &str, anchor: Option<&str>) -> Block {
        Block {
            id: id.to_string(),
            kind: BlockKind::Paragraph,
            text: text.to_string(),
            page: None,
            rect: None,
            anchor: anchor.map(str::to_string),
        }
    }

    #[test]
    fn side_only_flags_cover_both_directions() {
        let added = side_only_flags("", "phạt 5.000.000₫", ChangeKind::Added);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].rule_id, deltas::RULE_ENTITY_ADDED);

        let removed = side_only_flags("hạn 15/03/2026", "", ChangeKind::Removed);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].rule_id, deltas::RULE_ENTITY_REMOVED);
    }

    #[test]
    fn clause_rename_fires_on_anchor_change() {
        let old = vec![block("a0", "Điều 3: Bảo mật", Some("Điều 3"))];
        let new = vec![block("b0", "Điều 7: Bảo mật", Some("Điều 7"))];
        let mut changes = vec![Change {
            id: "c0".to_string(),
            kind: ChangeKind::Modified,
            old_block_ids: vec!["a0".to_string()],
            new_block_ids: vec!["b0".to_string()],
            old_page: None,
            new_page: None,
            spans: Vec::new(),
            similarity: None,
            move_delta: None,
            severity: None,
            flags: Vec::new(),
        }];
        let audit = enrich_changes(&mut changes, &old, &new);
        assert_eq!(audit.len(), 1);
        let rules: Vec<&str> = audit[0].rules.iter().map(|f| f.rule_id.as_str()).collect();
        assert!(
            rules.contains(&deltas::RULE_CLAUSE_RENAMED),
            "rules: {rules:?}"
        );
        assert!(changes[0].severity.is_some());
        let renamed = audit[0]
            .rules
            .iter()
            .find(|f| f.rule_id == deltas::RULE_CLAUSE_RENAMED)
            .unwrap();
        assert_eq!(renamed.old_value.as_deref(), Some("Điều 3"));
        assert_eq!(renamed.new_value.as_deref(), Some("Điều 7"));
    }

    #[test]
    fn equal_changes_are_not_enriched() {
        let blocks = vec![block("a0", "Văn bản giống nhau.", None)];
        let mut changes = vec![Change {
            id: "c0".to_string(),
            kind: ChangeKind::Equal,
            old_block_ids: vec!["a0".to_string()],
            new_block_ids: vec!["b0".to_string()],
            old_page: None,
            new_page: None,
            spans: Vec::new(),
            similarity: None,
            move_delta: None,
            severity: None,
            flags: Vec::new(),
        }];
        let audit = enrich_changes(&mut changes, &blocks, &blocks);
        assert!(audit.is_empty());
        assert!(changes[0].severity.is_none());
        assert!(changes[0].flags.is_empty());
    }
}
