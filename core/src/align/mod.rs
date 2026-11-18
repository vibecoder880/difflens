//! Alignment types and the 3-pass orchestration:
//!
//! 1. **Anchors** (deterministic): numbered clause headings, unique
//!    headings, unique long money strings → hard-constrained matched pairs.
//! 2. **Semantic**: cosine similarity matrix over embeddings → monotonic
//!    Needleman–Wunsch alignment with gap penalty, threshold-gated matches.
//!    Anchors win — NW never realigns them.
//! 3. **Move repair**: for blocks left unmatched by pass 2, window-search
//!    around the expected position for a repositioned counterpart.
//!
//! 100% deterministic: no randomness, no wall-clock dependence.

pub mod anchors;
pub mod needleman;

use serde::{Deserialize, Serialize};

use crate::embeddings::{Embedder, cosine};
use crate::error::{CoreError, CoreResult};
use crate::model::Block;

/// Tunables for the alignment engine. Defaults are the calibrated values
/// recorded in `core/tests/gold/manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignConfig {
    /// Minimum cosine similarity for NW to accept a pair.
    pub similarity_threshold: f32,
    /// NW gap penalty (applied per skipped block, weighted by block kind).
    pub gap_penalty: f32,
    /// Extra gap cost multiplier for headings (skipping a heading is more
    /// suspicious than skipping a paragraph).
    pub heading_gap_weight: f32,
    /// Move-repair search half-width around the expected position.
    pub move_window: usize,
    /// Minimum cosine similarity for a move-repair match.
    pub move_similarity_threshold: f32,
    /// Maximum blocks per NW matrix side; larger segments are chunked.
    pub max_chunk: usize,
}

impl Default for AlignConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.55,
            gap_penalty: -0.2,
            heading_gap_weight: 1.5,
            move_window: 3,
            move_similarity_threshold: 0.85,
            max_chunk: 2000,
        }
    }
}

/// How a matched pair was established (audit trail).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairSource {
    /// Pass 1: deterministic anchor (clause number / unique heading / money).
    Anchor,
    /// Pass 2: semantic NW alignment.
    Semantic,
    /// Pass 3: move repair.
    MoveRepair,
}

/// One aligned (old, new) block pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignedPair {
    /// Index into the old document's block list.
    pub old: usize,
    /// Index into the new document's block list.
    pub new: usize,
    /// Cosine similarity of the pair (embeddings), -1.0..=1.0.
    pub similarity: f32,
    /// Which pass established this pair.
    pub source: PairSource,
}

/// Result of aligning two documents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlignmentResult {
    /// Matched pairs, sorted by `(old, new)`.
    pub pairs: Vec<AlignedPair>,
    /// Old block indices that found no counterpart.
    pub unmatched_old: Vec<usize>,
    /// New block indices that found no counterpart.
    pub unmatched_new: Vec<usize>,
}

impl AlignmentResult {
    /// Pairs established by pass 3 (repositioned matches).
    pub fn moved_pairs(&self) -> impl Iterator<Item = &AlignedPair> + '_ {
        self.pairs
            .iter()
            .filter(|p| p.source == PairSource::MoveRepair)
    }

    /// Lookup: aligned new index for an old index.
    pub fn new_of(&self, old: usize) -> Option<usize> {
        self.pairs.iter().find(|p| p.old == old).map(|p| p.new)
    }
}

/// Align two block lists in three passes. Never panics on malformed input;
/// embedding failures surface as [`CoreError`].
pub fn align(old: &[Block], new: &[Block], embedder: &dyn Embedder) -> CoreResult<AlignmentResult> {
    align_with_config(old, new, embedder, &AlignConfig::default())
}

/// [`align`] with explicit tuning (tests, future tuning surfaces).
pub fn align_with_config(
    old: &[Block],
    new: &[Block],
    embedder: &dyn Embedder,
    config: &AlignConfig,
) -> CoreResult<AlignmentResult> {
    // Pass 1: anchors are hard constraints.
    let anchor_pairs = anchors::find_anchors(old, new);
    let mut anchored_old = vec![false; old.len()];
    let mut anchored_new = vec![false; new.len()];
    for (i, j) in &anchor_pairs {
        if *i < old.len() && *j < new.len() {
            anchored_old[*i] = true;
            anchored_new[*j] = true;
        }
    }

    // Pass 2: semantic alignment over everything the anchors left free.
    let old_texts: Vec<&str> = old.iter().map(|b| b.text.as_str()).collect();
    let new_texts: Vec<&str> = new.iter().map(|b| b.text.as_str()).collect();
    let old_vecs = embedder.embed(&old_texts)?;
    let new_vecs = embedder.embed(&new_texts)?;
    if old_vecs.len() != old.len() || new_vecs.len() != new.len() {
        return Err(CoreError::Align(format!(
            "embedder returned {} old / {} new vectors for {} / {} blocks",
            old_vecs.len(),
            new_vecs.len(),
            old.len(),
            new.len()
        )));
    }
    let similarity = |i: usize, j: usize| cosine(&old_vecs[i], &new_vecs[j]);

    let free_old: Vec<usize> = (0..old.len()).filter(|i| !anchored_old[*i]).collect();
    let free_new: Vec<usize> = (0..new.len()).filter(|j| !anchored_new[*j]).collect();
    let semantic_pairs =
        needleman::align_free_blocks(old, new, &free_old, &free_new, &similarity, config);

    let mut pairs: Vec<AlignedPair> = anchor_pairs
        .iter()
        .map(|(i, j)| AlignedPair {
            old: *i,
            new: *j,
            similarity: similarity(*i, *j),
            source: PairSource::Anchor,
        })
        .collect();
    pairs.extend(semantic_pairs.into_iter().map(|(i, j, s)| AlignedPair {
        old: i,
        new: j,
        similarity: s,
        source: PairSource::Semantic,
    }));

    // Pass 3: move repair over the leftovers.
    let mut used_old = vec![false; old.len()];
    let mut used_new = vec![false; new.len()];
    for p in &pairs {
        used_old[p.old] = true;
        used_new[p.new] = true;
    }
    let mut aligned_lookup: Vec<(usize, usize)> = pairs.iter().map(|p| (p.old, p.new)).collect();
    aligned_lookup.sort_unstable();

    let leftover_old: Vec<usize> = (0..old.len()).filter(|i| !used_old[*i]).collect();

    // 3a — local repair: within `move_window` of the interpolated expected
    // position, accept threshold-level matches NW missed (monotonicity
    // conflicts, boundary effects).
    for i in &leftover_old {
        if used_old[*i] {
            continue;
        }
        let expected = expected_new_position(*i, new.len(), &aligned_lookup);
        if let Some(j) =
            search_local_partner(*i, expected, new.len(), &used_new, &similarity, config)
        {
            used_new[j] = true;
            used_old[*i] = true;
            pairs.push(AlignedPair {
                old: *i,
                new: j,
                similarity: similarity(*i, j),
                source: PairSource::MoveRepair,
            });
        }
    }

    // 3b — displaced moves: mutual-best pairs over the still-unmatched sets
    // at the high move threshold. This catches long-distance repositioning
    // the local window cannot see.
    let moved = detect_displaced_moves(
        old.len(),
        new.len(),
        &used_old,
        &used_new,
        &similarity,
        config,
    );
    for (i, j, s) in moved {
        used_old[i] = true;
        used_new[j] = true;
        pairs.push(AlignedPair {
            old: i,
            new: j,
            similarity: s,
            source: PairSource::MoveRepair,
        });
    }

    let unmatched_old: Vec<usize> = (0..old.len()).filter(|i| !used_old[*i]).collect();
    let unmatched_new: Vec<usize> = (0..new.len()).filter(|j| !used_new[*j]).collect();

    pairs.sort_by_key(|p| (p.old, p.new));
    Ok(AlignmentResult {
        pairs,
        unmatched_old,
        unmatched_new,
    })
}

/// Where old block `i` "should" land in the new document, interpolated from
/// its nearest aligned neighbors; clamped into `[0, new_len)`.
fn expected_new_position(i: usize, new_len: usize, aligned: &[(usize, usize)]) -> f32 {
    let prev = aligned.iter().rev().find(|(o, _)| *o < i);
    let next = aligned.iter().find(|(o, _)| *o > i);
    let raw = match (prev, next) {
        (Some(&(o0, n0)), Some(&(o1, n1))) if o1 > o0 => {
            let t = (i - o0) as f32 / (o1 - o0) as f32;
            n0 as f32 + t * (n1 - n0) as f32
        }
        (Some(&(o0, n0)), _) => (n0 + (i - o0)) as f32,
        (None, Some(&(o1, n1))) => n1 as f32 - (o1 - i) as f32,
        (None, None) => i as f32,
    };
    raw.clamp(0.0, (new_len.saturating_sub(1)) as f32)
}

/// Find a local repair partner for unmatched old block `i` among still-unused
/// new blocks within `config.move_window` of the expected position. Requires
/// only the normal similarity threshold (NW already passed over these; the
/// window guards against accepting far-fetched pairs). Tie-breaking is
/// deterministic: similarity desc, distance to expected asc, new index asc.
fn search_local_partner(
    i: usize,
    expected: f32,
    new_len: usize,
    used_new: &[bool],
    similarity: &impl Fn(usize, usize) -> f32,
    config: &AlignConfig,
) -> Option<usize> {
    if new_len == 0 {
        return None;
    }
    let window = config.move_window as f32;
    let lo = ((expected - window).floor().max(0.0)) as usize;
    let hi = (((expected + window).ceil()) as usize).min(new_len.saturating_sub(1));

    let mut best: Option<(usize, f32, f32)> = None; // (j, sim, dist)
    for j in lo..=hi {
        if used_new.get(j).copied().unwrap_or(true) {
            continue;
        }
        let sim = similarity(i, j);
        if sim < config.similarity_threshold {
            continue;
        }
        let dist = (j as f32 - expected).abs();
        let take = match best {
            None => true,
            Some((bj, bs, bd)) => {
                sim > bs + 1e-9
                    || ((bs - sim) < 1e-9 && (dist < bd - 1e-9 || ((bd - dist) < 1e-9 && j < bj)))
            }
        };
        if take {
            best = Some((j, sim, dist));
        }
    }
    best.map(|(j, _, _)| j)
}

/// Mutual-best displaced matching over unmatched blocks: pair `i` and `j`
/// when they are each other's highest-similarity leftover partner and the
/// similarity reaches `config.move_similarity_threshold`. One-shot (computed
/// on a fixed leftover snapshot), O(U_old × U_new) similarity lookups.
fn detect_displaced_moves(
    old_len: usize,
    new_len: usize,
    used_old: &[bool],
    used_new: &[bool],
    similarity: &impl Fn(usize, usize) -> f32,
    config: &AlignConfig,
) -> Vec<(usize, usize, f32)> {
    let mut best_old: Vec<Option<(usize, f32)>> = vec![None; old_len]; // (new, sim)
    let mut best_new: Vec<Option<(usize, f32)>> = vec![None; new_len]; // (old, sim)

    let better = |s: f32, idx: usize, other: (usize, f32)| {
        s > other.1 + 1e-9 || ((other.1 - s) < 1e-9 && idx < other.0)
    };

    for i in 0..old_len {
        if used_old[i] {
            continue;
        }
        for j in 0..new_len {
            if used_new[j] {
                continue;
            }
            let s = similarity(i, j);
            if s < config.move_similarity_threshold {
                continue;
            }
            let replace_old = best_old[i].is_none_or(|o| better(s, j, o));
            if replace_old {
                best_old[i] = Some((j, s));
            }
            let replace_new = best_new[j].is_none_or(|o| better(s, i, o));
            if replace_new {
                best_new[j] = Some((i, s));
            }
        }
    }

    let mut out = Vec::new();
    for (i, entry) in best_old.iter().enumerate() {
        if let Some((j, s)) = *entry
            && best_new[j].is_some_and(|(bi, _)| bi == i)
        {
            out.push((i, j, s));
        }
    }
    out.sort_by_key(|(i, j, _)| (*i, *j));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::FakeHashEmbedder;
    use crate::model::BlockKind;

    pub(crate) fn block(text: &str) -> Block {
        Block {
            id: String::new(),
            kind: BlockKind::Paragraph,
            text: text.to_string(),
            page: None,
            rect: None,
            anchor: None,
        }
    }

    #[test]
    fn identical_documents_align_everything() {
        let doc: Vec<Block> = (0..6)
            .map(|k| block(&format!("block number {k} with words")))
            .collect();
        let result = align(&doc, &doc, &FakeHashEmbedder::default()).unwrap();
        assert_eq!(result.pairs.len(), 6);
        assert!(result.unmatched_old.is_empty());
        assert!(result.unmatched_new.is_empty());
        assert!(result.pairs.iter().all(|p| p.similarity > 0.99));
    }

    #[test]
    fn empty_documents_are_handled() {
        let e = FakeHashEmbedder::default();
        let empty: Vec<Block> = Vec::new();
        let doc = vec![block("hello")];
        assert!(align(&empty, &empty, &e).unwrap().pairs.is_empty());
        let r = align(&empty, &doc, &e).unwrap();
        assert!(r.pairs.is_empty() && r.unmatched_new == vec![0]);
        let r = align(&doc, &empty, &e).unwrap();
        assert!(r.pairs.is_empty() && r.unmatched_old == vec![0]);
    }

    #[test]
    fn pairs_are_sorted_and_one_to_one() {
        let old = vec![block("alpha"), block("beta"), block("gamma")];
        let new = vec![block("beta modified"), block("alpha"), block("gamma")];
        let r = align(&old, &new, &FakeHashEmbedder::default()).unwrap();
        for w in r.pairs.windows(2) {
            assert!(w[0].old < w[1].old);
        }
        let mut olds: Vec<usize> = r.pairs.iter().map(|p| p.old).collect();
        let mut news: Vec<usize> = r.pairs.iter().map(|p| p.new).collect();
        olds.sort_unstable();
        news.sort_unstable();
        olds.dedup();
        news.dedup();
        assert_eq!(olds.len(), r.pairs.len());
        assert_eq!(news.len(), r.pairs.len());
    }

    #[test]
    fn removed_and_added_are_reported() {
        // Truly dissimilar leftovers stay unmatched (the move-repair pass
        // only pairs leftovers above the high move threshold).
        let old = vec![
            block("xylophone quartet winds slowly"),
            block("shared block"),
        ];
        let new = vec![block("shared block"), block("nebular counters align")];
        let r = align(&old, &new, &FakeHashEmbedder::default()).unwrap();
        assert_eq!(r.unmatched_old, vec![0]);
        assert_eq!(r.unmatched_new, vec![1]);
        assert!(
            r.pairs
                .iter()
                .any(|p| p.old == 1 && p.new == 0 && p.similarity > 0.9)
        );
    }

    #[test]
    fn moved_block_is_detected_by_pass3() {
        let old = vec![
            block("alpha one"),
            block("beta two"),
            block("movable block text"),
            block("gamma three"),
        ];
        let new = vec![
            block("movable block text"),
            block("alpha one"),
            block("beta two"),
            block("gamma three"),
        ];
        let r = align(&old, &new, &FakeHashEmbedder::default()).unwrap();
        assert_eq!(r.pairs.len(), 4, "every block finds a partner");
        let moved: Vec<(usize, usize)> = r.moved_pairs().map(|p| (p.old, p.new)).collect();
        assert_eq!(moved, vec![(2, 0)]);
        assert!(r.pairs.iter().all(|p| p.similarity > 0.99));
    }

    #[test]
    fn expected_position_interpolates_from_neighbors() {
        let aligned = [(0usize, 0usize), (5usize, 9usize)];
        // Halfway between old 0 and old 5 → halfway between new 0 and 9.
        let e = expected_new_position(2, 12, &aligned);
        assert!((e - 3.6).abs() < 1e-4);
        // Before the first aligned pair → extrapolate backwards from it.
        let e = expected_new_position(0, 12, &aligned);
        assert!((e - 4.0).abs() < 1e-6);
        // No neighbors → identity.
        let e = expected_new_position(7, 12, &[]);
        assert_eq!(e, 7.0);
    }
}
