//! Pass 1 — deterministic anchors.
//!
//! Anchors are hard-constrained matched pairs established without
//! embeddings:
//!
//! 1. **Clause headings**: `Điều N`, `Article N`, `Mục N`, `Phụ lục`,
//!    and hierarchical numbers (`1.2.3`) — matched by their clause key when
//!    that key is unique in both documents.
//! 2. **Unique headings / long identical text**: exact normalized-text
//!    matches that occur exactly once per document (headings of any length,
//!    other blocks only when reasonably long).
//! 3. **Long money strings**: `12.500.000`, `1,234,567`, `12345678`, … —
//!    ≥ 7 digits, unique in both documents.
//!
//! Everything here is deterministic: pure regex + hash maps, no embeddings.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::model::Block;

/// Minimum digit count for a money string to qualify as an anchor.
const MONEY_MIN_DIGITS: usize = 7;
/// Minimum text length (chars) for non-heading exact-match anchors.
const TEXT_ANCHOR_MIN_CHARS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Priority {
    Clause = 0,
    Heading = 1,
    Money = 2,
}

/// Clause-number key at the start of a text, if any.
fn clause_key(text: &str) -> Option<String> {
    fn r(cell: &'static OnceLock<Regex>, f: impl FnOnce() -> Regex) -> &'static Regex {
        cell.get_or_init(f)
    }
    static DIEU: OnceLock<Regex> = OnceLock::new();
    static ARTICLE: OnceLock<Regex> = OnceLock::new();
    static MUC: OnceLock<Regex> = OnceLock::new();
    static PHULUC_SUFFIX: OnceLock<Regex> = OnceLock::new();
    static PHULUC_BARE: OnceLock<Regex> = OnceLock::new();
    static HIERARCHIC: OnceLock<Regex> = OnceLock::new();

    let lower = text.to_lowercase();
    let trimmed = lower.trim_start();

    if let Some(c) = r(&DIEU, || {
        Regex::new(r"(?i)^điều\s+(\d+[a-z0-9]*)\b").unwrap()
    })
    .captures(trimmed)
    {
        return Some(format!("điều {}", normalize_key(&c[1])));
    }
    if let Some(c) = r(&ARTICLE, || {
        Regex::new(r"(?i)^article\s+(\d+[a-z0-9]*)\b").unwrap()
    })
    .captures(trimmed)
    {
        return Some(format!("article {}", normalize_key(&c[1])));
    }
    if let Some(c) =
        r(&MUC, || Regex::new(r"(?i)^mục\s+(\d+[a-z0-9]*)\b").unwrap()).captures(trimmed)
    {
        return Some(format!("mục {}", normalize_key(&c[1])));
    }
    if let Some(c) = r(&PHULUC_SUFFIX, || {
        Regex::new(r"(?i)^phụ lục\s+([a-z0-9]+)\b").unwrap()
    })
    .captures(trimmed)
    {
        return Some(format!("phụ lục {}", normalize_key(&c[1])));
    }
    // `Phụ lục` without a suffix is its own key only when nothing follows.
    if r(&PHULUC_BARE, || Regex::new(r"(?i)^phụ lục$").unwrap()).is_match(trimmed) {
        return Some("phụ lục".to_string());
    }
    // Hierarchical numbering: 1.2, 1.2.3, 2.10.4 …
    if let Some(c) = r(&HIERARCHIC, || Regex::new(r"^(\d+(?:\.\d+)+)\b").unwrap()).captures(trimmed)
    {
        return Some(format!("#{}", normalize_key(&c[1])));
    }
    None
}

/// Money-ish substrings inside a text (group-separated or long digit runs).
fn money_matches(text: &str) -> Vec<String> {
    static MONEY: OnceLock<Regex> = OnceLock::new();
    let re = MONEY
        .get_or_init(|| Regex::new(r"\d{1,3}(?:[.,\s]\d{3})+(?:[.,]\d{1,2})?|\d{7,}").unwrap());
    re.find_iter(text)
        .map(|m| normalize_key(m.as_str()))
        .filter(|k| k.chars().filter(|c| c.is_ascii_digit()).count() >= MONEY_MIN_DIGITS)
        .collect()
}

/// Normalize an anchor key: collapse whitespace, strip surrounding
/// punctuation so `12.500.000,` and `12.500.000` anchor the same.
fn normalize_key(s: &str) -> String {
    s.trim()
        .trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '.'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text key for exact-match anchors (case-insensitive, whitespace-collapsed).
fn text_key(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Find anchor pairs: one-to-one, old indices ascending, deterministic.
pub fn find_anchors(old: &[Block], new: &[Block]) -> Vec<(usize, usize)> {
    let mut candidates: Vec<(Priority, usize, usize)> = Vec::new();

    // 1 + 2: per-block keys.
    collect_clause_and_text_anchors(old, new, &mut candidates);
    // 3: money strings.
    collect_money_anchors(old, new, &mut candidates);

    // Resolve conflicts deterministically: priority, then indices; enforce
    // one-to-one.
    candidates.sort();
    let mut used_old: HashMap<usize, ()> = HashMap::new();
    let mut used_new: HashMap<usize, ()> = HashMap::new();
    let mut pairs = Vec::new();
    for (_p, i, j) in candidates {
        if used_old.contains_key(&i) || used_new.contains_key(&j) {
            continue;
        }
        used_old.insert(i, ());
        used_new.insert(j, ());
        pairs.push((i, j));
    }
    pairs.sort();
    pairs
}

fn collect_clause_and_text_anchors(
    old: &[Block],
    new: &[Block],
    out: &mut Vec<(Priority, usize, usize)>,
) {
    let mut clause_index: HashMap<String, (Vec<usize>, Vec<usize>)> = HashMap::new();
    let mut text_index: HashMap<String, (Vec<usize>, Vec<usize>)> = HashMap::new();

    for (i, b) in old.iter().enumerate() {
        if let Some(k) = clause_key(&b.text) {
            clause_index.entry(k).or_default().0.push(i);
        }
        let eligible = matches!(b.kind, crate::model::BlockKind::Heading { .. })
            || b.text.chars().count() >= TEXT_ANCHOR_MIN_CHARS;
        if eligible {
            text_index.entry(text_key(&b.text)).or_default().0.push(i);
        }
    }
    for (j, b) in new.iter().enumerate() {
        if let Some(k) = clause_key(&b.text) {
            clause_index.entry(k).or_default().1.push(j);
        }
        let eligible = matches!(b.kind, crate::model::BlockKind::Heading { .. })
            || b.text.chars().count() >= TEXT_ANCHOR_MIN_CHARS;
        if eligible {
            text_index.entry(text_key(&b.text)).or_default().1.push(j);
        }
    }

    for (_k, (olds, news)) in clause_index {
        if olds.len() == 1 && news.len() == 1 {
            out.push((Priority::Clause, olds[0], news[0]));
        }
    }
    for (_k, (olds, news)) in text_index {
        if olds.len() == 1 && news.len() == 1 {
            out.push((Priority::Heading, olds[0], news[0]));
        }
    }
}

fn collect_money_anchors(old: &[Block], new: &[Block], out: &mut Vec<(Priority, usize, usize)>) {
    // Key → (occurrence counts, containing block indices) per document.
    let mut old_index: HashMap<String, (usize, Vec<usize>)> = HashMap::new();
    let mut new_index: HashMap<String, (usize, Vec<usize>)> = HashMap::new();
    for (i, b) in old.iter().enumerate() {
        for key in money_matches(&b.text) {
            let e = old_index.entry(key).or_default();
            e.0 += 1;
            if !e.1.last().is_some_and(|&last| last == i) {
                e.1.push(i);
            }
        }
    }
    for (j, b) in new.iter().enumerate() {
        for key in money_matches(&b.text) {
            let e = new_index.entry(key).or_default();
            e.0 += 1;
            if !e.1.last().is_some_and(|&last| last == j) {
                e.1.push(j);
            }
        }
    }

    // Keys stored as owned normalized strings — exact-match join across docs.
    for (old_key, (old_count, old_blocks)) in &old_index {
        if *old_count != 1 {
            continue;
        }
        if let Some((1, new_blocks)) = new_index.get(old_key) {
            out.push((Priority::Money, old_blocks[0], new_blocks[0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BlockKind;

    fn block(kind: BlockKind, text: &str) -> Block {
        Block {
            id: String::new(),
            kind,
            text: text.to_string(),
            page: None,
            rect: None,
            anchor: None,
        }
    }

    #[test]
    fn clause_keys_extract() {
        assert_eq!(clause_key("Điều 5: Thanh toán"), Some("điều 5".into()));
        assert_eq!(clause_key("điều 12a"), Some("điều 12a".into()));
        assert_eq!(
            clause_key("Article 3 Obligations"),
            Some("article 3".into())
        );
        assert_eq!(clause_key("Mục 2 Chi tiết"), Some("mục 2".into()));
        assert_eq!(clause_key("Phụ lục I Danh sách"), Some("phụ lục i".into()));
        assert_eq!(clause_key("Phụ lục"), Some("phụ lục".into()));
        assert_eq!(clause_key("1.2.3 Sub clause"), Some("#1.2.3".into()));
        assert_eq!(clause_key("2.10 Fees"), Some("#2.10".into()));
        assert_eq!(clause_key("5. Plain number no dot chain"), None);
        assert_eq!(clause_key("Just a paragraph"), None);
    }

    #[test]
    fn money_matches_long_strings_only() {
        assert_eq!(
            money_matches("total of 12.500.000 VND and 1,234,567 USD"),
            vec!["12.500.000", "1,234,567"]
        );
        assert_eq!(money_matches("12345678 id"), vec!["12345678"]);
        assert!(money_matches("small 100 and 30 days").is_empty());
        assert!(money_matches("1.234").is_empty()); // only 4 digits
    }

    #[test]
    fn clause_heading_anchors_pair() {
        let old = vec![
            block(BlockKind::Paragraph, "intro"),
            block(BlockKind::Heading { level: 1 }, "Điều 5. Thanh toán"),
            block(BlockKind::Paragraph, "body five"),
            block(BlockKind::Heading { level: 1 }, "Điều 6. Bảo mật"),
        ];
        let new = vec![
            block(
                BlockKind::Heading { level: 1 },
                "Điều 5. Quy định thanh toán",
            ),
            block(BlockKind::Paragraph, "body five changed"),
            block(BlockKind::Heading { level: 1 }, "Điều 6. Bảo mật thông tin"),
        ];
        let pairs = find_anchors(&old, &new);
        assert_eq!(pairs, vec![(1, 0), (3, 2)]);
    }

    #[test]
    fn unique_heading_anchors_pair() {
        let old = vec![
            block(BlockKind::Heading { level: 1 }, "Warranty Terms"),
            block(BlockKind::Heading { level: 1 }, "Other"),
        ];
        let new = vec![
            block(BlockKind::Heading { level: 1 }, "Intro"),
            block(BlockKind::Heading { level: 1 }, "Warranty Terms"),
            block(BlockKind::Heading { level: 1 }, "Other"),
        ];
        let pairs = find_anchors(&old, &new);
        assert!(pairs.contains(&(0, 1)));
        // "Other" appears once per doc too.
        assert!(pairs.contains(&(1, 2)));
    }

    #[test]
    fn money_anchor_pairs_unique_strings() {
        let m1 = "Tổng cộng: 12.500.000 đồng";
        let old = vec![
            block(BlockKind::Paragraph, m1),
            block(BlockKind::Paragraph, "filler one"),
        ];
        let new = vec![
            block(BlockKind::Paragraph, "filler two"),
            block(BlockKind::Paragraph, "Số tiền là 12.500.000 (VNĐ)"),
        ];
        let pairs = find_anchors(&old, &new);
        assert_eq!(pairs, vec![(0, 1)]);
    }

    #[test]
    fn repeated_money_is_not_an_anchor() {
        let old = vec![
            block(BlockKind::Paragraph, "pay 1.500.000 now"),
            block(BlockKind::Paragraph, "pay 1.500.000 later"),
        ];
        let new = vec![block(BlockKind::Paragraph, "pay 1.500.000 anytime")];
        assert!(find_anchors(&old, &new).is_empty());
    }

    #[test]
    fn duplicate_clause_number_is_not_an_anchor() {
        let h = |t| block(BlockKind::Heading { level: 1 }, t);
        let old = vec![h("Điều 5. A"), h("Điều 5. B")];
        let new = vec![h("Điều 5. C")];
        assert!(find_anchors(&old, &new).is_empty());
    }

    #[test]
    fn clause_wins_over_money_conflict() {
        // Same block matched by both clause key and money key → clause wins,
        // one-to-one enforced.
        let old = vec![
            block(BlockKind::Heading { level: 1 }, "Điều 7. Phí 35.000.000"),
            block(BlockKind::Paragraph, "other 35.000.000 mention"),
        ];
        let new = vec![
            block(BlockKind::Paragraph, "other 35.000.000 mention moved"),
            block(
                BlockKind::Heading { level: 1 },
                "Điều 7. Phí 35.000.000 đồng",
            ),
        ];
        let pairs = find_anchors(&old, &new);
        // Điều 7 pair must exist; money cannot double-book either side.
        assert!(pairs.contains(&(0, 1)));
        assert_eq!(pairs.len(), 1);
    }
}
