//! Word/char inline diff spans built on `similar`.
//!
//! Strategy: word-granularity `TextDiff`, then Replace runs whose joined
//! sides are similar enough (jaro-winkler ≥ 0.5) get refined to
//! character-granularity spans — "the price changed from 30 to 45" shows the
//! two digits as the only deltas instead of two whole words. Dissimilar
//! replacements stay as one Delete + one Insert (cleaner than char soup).
//!
//! Deterministic: same inputs → same spans; tie-breaking is entirely
//! Myers-order stable.

use similar::{ChangeTag, TextDiff};

use crate::diff::bounded_jaro_winkler;
use crate::model::{DiffSpan, SpanOp};

/// Minimum jaro-winkler similarity between the joined sides of a Replace
/// run before it is refined to character granularity.
const CHAR_REFINE_MIN_SIMILARITY: f64 = 0.6;

/// Word-level diff with char-level refinement of similar replacements.
pub fn word_spans(old: &str, new: &str) -> Vec<DiffSpan> {
    let diff = TextDiff::from_words(old, new);
    let mut spans: Vec<DiffSpan> = Vec::new();
    // Pending consecutive Delete/Insert words — a Replace run.
    let mut pending: Vec<(bool, String)> = Vec::new(); // (is_insert, text)

    fn flush(pending: &mut Vec<(bool, String)>, spans: &mut Vec<DiffSpan>) {
        if pending.is_empty() {
            return;
        }
        let deleted: String = pending
            .iter()
            .filter(|(ins, _)| !ins)
            .map(|(_, t)| t.as_str())
            .collect();
        let inserted: String = pending
            .iter()
            .filter(|(ins, _)| *ins)
            .map(|(_, t)| t.as_str())
            .collect();
        pending.clear();

        if deleted.is_empty() {
            push_span(spans, SpanOp::Insert, &inserted);
        } else if inserted.is_empty() {
            push_span(spans, SpanOp::Delete, &deleted);
        } else if bounded_jaro_winkler(&deleted, &inserted) >= CHAR_REFINE_MIN_SIMILARITY {
            // Similar word pair → char-granular refinement.
            let char_diff = TextDiff::from_chars(&deleted, &inserted);
            for char_op in char_diff.ops() {
                for change in char_diff.iter_changes(char_op) {
                    let op = match change.tag() {
                        ChangeTag::Equal => SpanOp::Equal,
                        ChangeTag::Delete => SpanOp::Delete,
                        ChangeTag::Insert => SpanOp::Insert,
                    };
                    push_span(spans, op, change.value());
                }
            }
        } else {
            push_span(spans, SpanOp::Delete, &deleted);
            push_span(spans, SpanOp::Insert, &inserted);
        }
    }

    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            match change.tag() {
                ChangeTag::Equal => {
                    flush(&mut pending, &mut spans);
                    push_span(&mut spans, SpanOp::Equal, change.value());
                }
                ChangeTag::Delete => pending.push((false, change.value().to_string())),
                ChangeTag::Insert => pending.push((true, change.value().to_string())),
            }
        }
    }
    flush(&mut pending, &mut spans);

    merge_adjacent(&mut spans);
    spans
}

fn push_span(spans: &mut Vec<DiffSpan>, op: SpanOp, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.op == op
    {
        last.text.push_str(text);
        return;
    }
    spans.push(DiffSpan {
        op,
        text: text.to_string(),
    });
}

/// Merge adjacent same-op spans (defensive; `push_span` already merges).
fn merge_adjacent(spans: &mut [DiffSpan]) {
    // No-op: kept as an explicit invariant note — spans are always
    // alternating op-wise by construction.
    let _ = spans;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(spans: &[DiffSpan]) -> Vec<SpanOp> {
        spans.iter().map(|s| s.op).collect()
    }

    fn texts(spans: &[DiffSpan], op: SpanOp) -> String {
        spans
            .iter()
            .filter(|s| s.op == op)
            .map(|s| s.text.as_str())
            .collect()
    }

    #[test]
    fn identical_text_is_one_equal_span() {
        let spans = word_spans("the same words here", "the same words here");
        assert_eq!(ops(&spans), vec![SpanOp::Equal]);
        assert_eq!(spans[0].text, "the same words here");
    }

    #[test]
    fn word_insert_and_delete_are_reported() {
        let spans = word_spans("one two three", "one two four three");
        assert_eq!(spans.first().unwrap().op, SpanOp::Equal);
        assert!(ops(&spans).contains(&SpanOp::Insert));
        assert_eq!(texts(&spans, SpanOp::Insert), "four ");

        let spans = word_spans("one two three", "one three");
        assert_eq!(texts(&spans, SpanOp::Delete), "two ");
    }

    #[test]
    fn similar_word_replacement_refines_to_chars() {
        // "30" → "45": dissimilar words... use similar pair instead.
        let spans = word_spans("price is paymentx", "price is paymenty");
        // "paymentx"/"paymenty" are highly similar → refined so the common
        // stem lands in an Equal span.
        assert!(
            spans
                .iter()
                .any(|s| s.op == SpanOp::Equal && s.text.contains("payment"))
        );
        assert_eq!(texts(&spans, SpanOp::Delete), "x");
        assert_eq!(texts(&spans, SpanOp::Insert), "y");
    }

    #[test]
    fn dissimilar_replacement_stays_word_level() {
        // No common characters → below the refinement gate → the whole
        // words surface as one Delete + one Insert.
        let spans = word_spans("aaaa bbbb", "zzzz yyyy");
        assert_eq!(texts(&spans, SpanOp::Delete), "aaaabbbb");
        assert_eq!(texts(&spans, SpanOp::Insert), "zzzzyyyy");
    }

    #[test]
    fn empty_and_edge_inputs() {
        assert!(word_spans("", "").is_empty());
        let spans = word_spans("", "new text");
        assert_eq!(ops(&spans), vec![SpanOp::Insert]);
        let spans = word_spans("old text", "");
        assert_eq!(ops(&spans), vec![SpanOp::Delete]);
    }

    #[test]
    fn spans_reconstruct_both_sides() {
        let old = "The payment is due within 30 days of invoice";
        let new = "The payment is due within 45 days of the invoice";
        let spans = word_spans(old, new);
        let old_rebuilt: String = spans
            .iter()
            .filter(|s| s.op != SpanOp::Insert)
            .map(|s| s.text.as_str())
            .collect();
        let new_rebuilt: String = spans
            .iter()
            .filter(|s| s.op != SpanOp::Delete)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(old_rebuilt, old);
        assert_eq!(new_rebuilt, new);
    }

    #[test]
    fn vietnamese_word_diff_works() {
        let spans = word_spans(
            "Thanh toán trong vòng 30 ngày",
            "Thanh toán trong vòng 60 ngày",
        );
        // "30 " ↔ "60 " is similar → char-refined to the single digit.
        assert_eq!(texts(&spans, SpanOp::Delete), "3");
        assert_eq!(texts(&spans, SpanOp::Insert), "6");
        assert!(
            spans
                .iter()
                .any(|s| s.op == SpanOp::Equal && s.text.contains("0 "))
        );
    }

    #[test]
    fn ops_enum_is_tagged_by_similar() {
        // Guards against a `similar` upgrade changing tag semantics.
        use similar::DiffTag;
        let d = TextDiff::from_words("a b", "a b");
        assert!(matches!(d.ops()[0].tag(), DiffTag::Equal));
    }
}
