//! Diff engine: turns an [`AlignmentResult`](crate::align::AlignmentResult)
//! pair set into typed [`Change`]s with inline spans.
//!
//! Classification per matched pair:
//! - identical text at the same ordinal → [`ChangeKind::Equal`]
//! - identical text at a displaced ordinal (≥ [`MOVE_DISPLACEMENT_MIN`]) →
//!   [`ChangeKind::Moved`], with `move_delta` when both blocks carry rects
//! - otherwise → [`ChangeKind::Modified`], with word/char spans and a
//!   text-similarity score (jaro-winkler)
//!
//! Unmatched old blocks become `Removed`, unmatched new blocks `Added`.
//! Changes are ordered deterministically by `(old ordinal, new ordinal)`
//! (missing sides sort last) and receive stable ids `c0..cN` in that order.

pub mod word;

pub use word::word_spans;

use crate::model::{Block, Change, ChangeKind, DiffSpan, DiffStats, SpanOp};

/// Minimum ordinal displacement for an identical-text pair to count as
/// moved rather than equal (±1 shifts from a single adjacent insert/delete
/// are not moves).
const MOVE_DISPLACEMENT_MIN: usize = 2;

/// Diff two block lists over the given alignment pairs `(old_idx, new_idx)`.
///
/// Malformed pairs (out-of-range indices, duplicates) are skipped instead of
/// panicking; every unmatched block still yields Added/Removed entries.
pub fn diff_blocks(old: &[Block], new: &[Block], pairs: &[(usize, usize)]) -> Vec<Change> {
    let mut matched_old = vec![false; old.len()];
    let mut matched_new = vec![false; new.len()];

    let mut entries: Vec<(Option<usize>, Option<usize>, ChangeKind)> = Vec::new();
    for &(i, j) in pairs {
        let (Some(ob), Some(nb)) = (old.get(i), new.get(j)) else {
            continue;
        };
        // One-to-one guard: a duplicated pair would double-count blocks.
        if matched_old[i] || matched_new[j] {
            continue;
        }
        matched_old[i] = true;
        matched_new[j] = true;

        let kind = if ob.text == nb.text {
            if i.abs_diff(j) >= MOVE_DISPLACEMENT_MIN {
                ChangeKind::Moved
            } else {
                ChangeKind::Equal
            }
        } else {
            ChangeKind::Modified
        };
        entries.push((Some(i), Some(j), kind));
    }
    for (i, _) in old.iter().enumerate().filter(|(i, _)| !matched_old[*i]) {
        entries.push((Some(i), None, ChangeKind::Removed));
    }
    for (j, _) in new.iter().enumerate().filter(|(j, _)| !matched_new[*j]) {
        entries.push((None, Some(j), ChangeKind::Added));
    }
    entries.sort_by_key(|(o, n, _)| (o.unwrap_or(usize::MAX), n.unwrap_or(usize::MAX)));

    entries
        .into_iter()
        .enumerate()
        .map(|(idx, (o, n, kind))| build_change(idx, kind, o, n, old, new))
        .collect()
}

/// Summary counts by change kind.
pub fn summarize(changes: &[Change]) -> DiffStats {
    let mut stats = DiffStats::default();
    for c in changes {
        match c.kind {
            ChangeKind::Added => stats.added += 1,
            ChangeKind::Removed => stats.removed += 1,
            ChangeKind::Modified => stats.modified += 1,
            ChangeKind::Moved => stats.moved += 1,
            ChangeKind::Equal => stats.equal += 1,
        }
    }
    stats
}

fn build_change(
    idx: usize,
    kind: ChangeKind,
    o: Option<usize>,
    n: Option<usize>,
    old: &[Block],
    new: &[Block],
) -> Change {
    let ob = o.map(|i| &old[i]);
    let nb = n.map(|j| &new[j]);

    let spans = match kind {
        ChangeKind::Modified => word_spans(
            ob.map(|b| b.text.as_str()).unwrap_or_default(),
            nb.map(|b| b.text.as_str()).unwrap_or_default(),
        ),
        // Kept for complete pair mapping on moves (identical content).
        ChangeKind::Moved => ob
            .map(|b| DiffSpan {
                op: SpanOp::Equal,
                text: b.text.clone(),
            })
            .into_iter()
            .collect(),
        ChangeKind::Equal | ChangeKind::Added | ChangeKind::Removed => Vec::new(),
    };

    let similarity = match kind {
        ChangeKind::Equal => Some(1.0),
        ChangeKind::Modified | ChangeKind::Moved => {
            let (a, b) = (
                ob.map(|b| b.text.as_str()).unwrap_or_default(),
                nb.map(|b| b.text.as_str()).unwrap_or_default(),
            );
            Some(strsim::jaro_winkler(a, b) as f32)
        }
        ChangeKind::Added | ChangeKind::Removed => None,
    };

    let move_delta = match kind {
        ChangeKind::Moved => match (ob.and_then(|b| b.rect), nb.and_then(|b| b.rect)) {
            (Some(ra), Some(rb)) => Some((rb.x - ra.x, rb.y - ra.y)),
            _ => None,
        },
        _ => None,
    };

    // Side-only changes carry their full block text (no spans exist to
    // render); paired changes keep the text out of the payload entirely.
    let (old_text, new_text) = match kind {
        ChangeKind::Added => (None, nb.map(|b| b.text.clone())),
        ChangeKind::Removed => (ob.map(|b| b.text.clone()), None),
        _ => (None, None),
    };

    Change {
        id: format!("c{idx}"),
        kind,
        old_block_ids: ob.map(|b| vec![b.id.clone()]).unwrap_or_default(),
        new_block_ids: nb.map(|b| vec![b.id.clone()]).unwrap_or_default(),
        old_page: ob.and_then(|b| b.page),
        new_page: nb.and_then(|b| b.page),
        spans,
        old_text,
        new_text,
        similarity,
        move_delta,
        // Set by heuristics in a later phase.
        severity: None,
        flags: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockKind, Rect};

    fn block(text: &str) -> Block {
        Block {
            id: String::new(),
            kind: BlockKind::Paragraph,
            text: text.to_string(),
            page: None,
            rect: None,
            anchor: None,
        }
    }

    fn with_id(mut b: Block, id: &str) -> Block {
        b.id = id.to_string();
        b
    }

    #[test]
    fn modified_pair_gets_spans_and_similarity() {
        let old = [with_id(block("payment within 30 days"), "a0")];
        let new = [with_id(block("payment within 45 days"), "b0")];
        let changes = diff_blocks(&old, &new, &[(0, 0)]);
        assert_eq!(changes.len(), 1);
        let c = &changes[0];
        assert_eq!(c.id, "c0");
        assert_eq!(c.kind, ChangeKind::Modified);
        assert!(c.spans.iter().any(|s| s.op == SpanOp::Delete));
        assert!(c.spans.iter().any(|s| s.op == SpanOp::Insert));
        assert!(c.similarity.unwrap() > 0.8);
        assert_eq!(c.old_block_ids, vec!["a0"]);
        assert_eq!(c.new_block_ids, vec!["b0"]);
    }

    #[test]
    fn equal_and_moved_classification() {
        let old = [
            block("alpha"),
            block("beta"),
            block("gamma"),
            block("delta"),
        ];
        let new = [
            block("delta"),
            block("beta"),
            block("gamma"),
            block("alpha"),
        ];
        // alpha: 0→3 (displaced ≥2 → moved), beta/gamma in place, delta 3→0.
        let pairs = [(0, 3), (1, 1), (2, 2), (3, 0)];
        let changes = diff_blocks(&old, &new, &pairs);
        let kinds: Vec<ChangeKind> = changes.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ChangeKind::Moved,
                ChangeKind::Equal,
                ChangeKind::Equal,
                ChangeKind::Moved
            ]
        );
        assert_eq!(changes[0].spans.len(), 1);
        assert_eq!(changes[0].spans[0].op, SpanOp::Equal);
    }

    #[test]
    fn small_shift_is_equal_not_moved() {
        let old = [block("gone"), block("shared text")];
        let new = [block("shared text")];
        let changes = diff_blocks(&old, &new, &[(1, 0)]);
        // Ordering is by old ordinal: the Removed entry (old 0) sorts first.
        let kinds: Vec<ChangeKind> = changes.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, vec![ChangeKind::Removed, ChangeKind::Equal]);
    }

    #[test]
    fn added_removed_and_ordering() {
        let old = [
            with_id(block("stay"), "a0"),
            with_id(block("removed"), "a1"),
            with_id(block("stay too"), "a2"),
        ];
        let new = [
            with_id(block("stay"), "b0"),
            with_id(block("stay too"), "b1"),
            with_id(block("added"), "b2"),
        ];
        let changes = diff_blocks(&old, &new, &[(0, 0), (2, 1)]);
        let kinds: Vec<(String, ChangeKind)> =
            changes.iter().map(|c| (c.id.clone(), c.kind)).collect();
        // Deterministic ordering by (old ordinal, new ordinal): the Removed
        // entry (old ordinal 1) sorts between the two paired changes, Added
        // (no old side) sorts last.
        assert_eq!(
            kinds,
            vec![
                ("c0".into(), ChangeKind::Equal),
                ("c1".into(), ChangeKind::Removed),
                ("c2".into(), ChangeKind::Equal),
                ("c3".into(), ChangeKind::Added),
            ]
        );
        assert_eq!(changes[1].old_block_ids, vec!["a1"]);
        assert!(changes[1].new_block_ids.is_empty());
        assert_eq!(changes[3].new_block_ids, vec!["b2"]);
        assert!(changes[3].similarity.is_none());
        // Side-only changes carry the full block text on their side.
        assert_eq!(changes[1].old_text.as_deref(), Some("removed"));
        assert!(changes[1].new_text.is_none());
        assert!(changes[3].old_text.is_none());
        assert_eq!(changes[3].new_text.as_deref(), Some("added"));
    }

    #[test]
    fn paired_changes_omit_side_text() {
        // Modified/Moved/Equal keep spans and serialize without side text.
        let old = [block("payment within 30 days")];
        let new = [block("payment within 45 days")];
        let changes = diff_blocks(&old, &new, &[(0, 0)]);
        assert!(changes[0].old_text.is_none());
        assert!(changes[0].new_text.is_none());
        assert!(!changes[0].spans.is_empty());

        let json = serde_json::to_string(&changes[0]).unwrap();
        assert!(!json.contains("old_text"), "json: {json}");
        assert!(!json.contains("new_text"), "json: {json}");

        let equal_old = [block("same")];
        let equal_new = [block("same")];
        let equal = diff_blocks(&equal_old, &equal_new, &[(0, 0)]);
        assert!(equal[0].old_text.is_none() && equal[0].new_text.is_none());
    }

    #[test]
    fn move_delta_from_rects() {
        let mut k0 = block("keeper zero");
        k0.rect = Some(Rect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 30.0,
        });
        let mut k1 = block("keeper one");
        k1.rect = Some(Rect {
            x: 10.0,
            y: 60.0,
            width: 100.0,
            height: 30.0,
        });
        let mut moved = block("moved content");
        moved.rect = Some(Rect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 30.0,
        });
        let mut moved_new = block("moved content");
        moved_new.rect = Some(Rect {
            x: 15.0,
            y: 200.0,
            width: 100.0,
            height: 30.0,
        });
        // moved: old ordinal 2 → new ordinal 0 (displaced ≥ 2 → Moved).
        let changes = diff_blocks(
            &[k0, k1, moved],
            &[moved_new, block("keeper zero"), block("keeper one")],
            &[(2, 0), (0, 1), (1, 2)],
        );
        // Sorted by old ordinal: (0,1) c0 Equal, (1,2) c1 Equal, (2,0) c2 Moved.
        assert_eq!(changes[2].kind, ChangeKind::Moved);
        assert_eq!(changes[2].move_delta, Some((5.0, 180.0)));
    }

    #[test]
    fn malformed_pairs_are_skipped_without_panicking() {
        let old = [block("a")];
        let new = [block("b"), block("c")];
        // Out-of-range + duplicated pairs must not panic.
        let changes = diff_blocks(&old, &new, &[(0, 5), (9, 0), (0, 0), (0, 1)]);
        // (0,0) wins; the duplicate (0,1) is dropped; (9,0),(0,5) skipped;
        // new[1] "c" stays unmatched → Added.
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        assert_eq!(changes[1].kind, ChangeKind::Added);
    }

    #[test]
    fn empty_inputs_produce_empty_output() {
        assert!(diff_blocks(&[], &[], &[]).is_empty());
        let new = [block("only new")];
        let changes = diff_blocks(&[], &new, &[]);
        assert_eq!(changes[0].kind, ChangeKind::Added);
    }

    #[test]
    fn summarize_counts_kinds() {
        let old = [block("a"), block("b")];
        let new = [block("a"), block("c"), block("d")];
        let changes = diff_blocks(&old, &new, &[(0, 0), (1, 1)]);
        let stats = summarize(&changes);
        assert_eq!(stats.equal, 1);
        assert_eq!(stats.modified, 1);
        assert_eq!(stats.added, 1);
        assert_eq!(stats.removed, 0);
        assert_eq!(stats.moved, 0);
    }
}
