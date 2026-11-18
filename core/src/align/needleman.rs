//! Pass 2 — semantic alignment: Needleman–Wunsch over the cosine similarity
//! matrix of the blocks the anchor pass left free.
//!
//! The matrix is O(n²) in memory; both sides are chunked at
//! [`AlignConfig::max_chunk`] (default 2000 blocks → ~16 MB per f32 matrix)
//! so 1,000+ paragraph documents stay within the 6 GB VM budget. Chunk
//! windows never mix: pairs do not cross window boundaries (anchors bound
//! the drift this can cause on very large documents).
//!
//! Matches are gated: a diagonal move is only allowed when the pair
//! similarity reaches `AlignConfig::similarity_threshold`, so NW picks the
//! best *monotonic* selection of acceptable pairs under kind-weighted gap
//! penalties. Deterministic tie-breaking mirrors the forward pass:
//! diagonal, then skip-old, then skip-new.

use crate::model::{Block, BlockKind};

use super::AlignConfig;

/// Align the free (anchor-unmatched) block index sets monotonically.
/// Returns `(old_index, new_index, similarity)` triples with `old` ascending.
pub fn align_free_blocks(
    old: &[Block],
    new: &[Block],
    free_old: &[usize],
    free_new: &[usize],
    similarity: &impl Fn(usize, usize) -> f32,
    config: &AlignConfig,
) -> Vec<(usize, usize, f32)> {
    if free_old.is_empty() || free_new.is_empty() {
        return Vec::new();
    }

    let chunk = config.max_chunk.max(1);
    let mut out = Vec::new();
    // Lockstep windows: pairs never cross a chunk boundary. When the two
    // documents differ hugely in block count past 2000, trailing chunks have
    // no counterpart window and stay unpaired (anchors bound the drift).
    for (old_window, new_window) in free_old.chunks(chunk).zip(free_new.chunks(chunk)) {
        nw_window(
            old, new, old_window, new_window, similarity, config, &mut out,
        );
    }
    out.sort_by_key(|(i, j, _)| (*i, *j));
    out
}

/// Weighted gap cost for skipping one block (negative; headings cost more).
fn gap_cost(block: &Block, config: &AlignConfig) -> f32 {
    let weight = match block.kind {
        BlockKind::Heading { .. } => config.heading_gap_weight,
        _ => 1.0,
    };
    config.gap_penalty * weight
}

/// Classic full-matrix NW on one window, with threshold-gated diagonals.
fn nw_window(
    old: &[Block],
    new: &[Block],
    old_window: &[usize],
    new_window: &[usize],
    similarity: &impl Fn(usize, usize) -> f32,
    config: &AlignConfig,
    out: &mut Vec<(usize, usize, f32)>,
) {
    let m = old_window.len();
    let n = new_window.len();
    if m == 0 || n == 0 {
        return;
    }

    // Similarity cache for allowed diagonals only (cells below the
    // threshold store None and can never be matched).
    let mut sim_cache: Vec<Option<f32>> = vec![None; m * n];
    for i in 0..m {
        for j in 0..n {
            let s = similarity(old_window[i], new_window[j]);
            if s >= config.similarity_threshold {
                sim_cache[i * n + j] = Some(s);
            }
        }
    }

    // scores[i * width + j]: best cumulative score for prefixes
    // (old_window[..i], new_window[..j]).
    let width = n + 1;
    let mut scores = vec![0.0f32; (m + 1) * width];
    for i in 1..=m {
        scores[i * width] = scores[(i - 1) * width] + gap_cost(&old[old_window[i - 1]], config);
    }
    for j in 1..=n {
        scores[j] = scores[j - 1] + gap_cost(&new[new_window[j - 1]], config);
    }

    for i in 1..=m {
        for j in 1..=n {
            // Diagonal (match) — only when the pair clears the threshold.
            let mut best = match sim_cache[(i - 1) * n + (j - 1)] {
                Some(s) => scores[(i - 1) * width + (j - 1)] + s,
                None => f32::NEG_INFINITY,
            };
            // Skip old block i-1.
            let up = scores[(i - 1) * width + j] + gap_cost(&old[old_window[i - 1]], config);
            if up > best {
                best = up;
            }
            // Skip new block j-1.
            let left = scores[i * width + (j - 1)] + gap_cost(&new[new_window[j - 1]], config);
            if left > best {
                best = left;
            }
            scores[i * width + j] = best;
        }
    }

    // Traceback; identical recomputation gives exact float comparisons, so
    // the preference order is deterministic: diag, then up, then left.
    let mut i = m;
    let mut j = n;
    let mut pairs = Vec::new();
    while i > 0 && j > 0 {
        let up = scores[(i - 1) * width + j] + gap_cost(&old[old_window[i - 1]], config);
        let left = scores[i * width + (j - 1)] + gap_cost(&new[new_window[j - 1]], config);

        if let Some(s) = sim_cache[(i - 1) * n + (j - 1)] {
            let diag = scores[(i - 1) * width + (j - 1)] + s;
            if diag >= up && diag >= left {
                pairs.push((old_window[i - 1], new_window[j - 1], s));
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if up >= left {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    out.extend(pairs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::{Embedder, FakeHashEmbedder, cosine};
    use crate::model::BlockKind;

    fn paragraph(text: &str) -> Block {
        Block {
            id: String::new(),
            kind: BlockKind::Paragraph,
            text: text.to_string(),
            page: None,
            rect: None,
            anchor: None,
        }
    }

    fn setup<'a>(
        old_texts: &'a [&'a str],
        new_texts: &'a [&'a str],
    ) -> impl Fn(usize, usize) -> f32 + 'a {
        let e = FakeHashEmbedder::default();
        let ov = Embedder::embed(&e, old_texts).unwrap();
        let nv = Embedder::embed(&e, new_texts).unwrap();
        move |i: usize, j: usize| cosine(&ov[i], &nv[j])
    }

    #[test]
    fn nw_prefers_matches_over_gaps() {
        let old = ["alpha block", "beta block", "gamma block"];
        let new = ["alpha block", "gamma block"];
        let sim = setup(&old, &new);
        let ob: Vec<Block> = old.iter().map(|t| paragraph(t)).collect();
        let nb: Vec<Block> = new.iter().map(|t| paragraph(t)).collect();

        let pairs = align_free_blocks(&ob, &nb, &[0, 1, 2], &[0, 1], &sim, &AlignConfig::default());
        // alpha and gamma match; beta is gapped.
        assert_eq!(pairs.len(), 2);
        assert_eq!((pairs[0].0, pairs[0].1), (0, 0));
        assert_eq!((pairs[1].0, pairs[1].1), (2, 1));
        assert!(pairs[0].2 > 0.99);
    }

    #[test]
    fn nw_is_monotonic() {
        let old = ["one", "two", "three", "four", "five"];
        let new = ["one", "three", "five"];
        let sim = setup(&old, &new);
        let ob: Vec<Block> = old.iter().map(|t| paragraph(t)).collect();
        let nb: Vec<Block> = new.iter().map(|t| paragraph(t)).collect();

        let pairs = align_free_blocks(
            &ob,
            &nb,
            &[0, 1, 2, 3, 4],
            &[0, 1, 2],
            &sim,
            &AlignConfig::default(),
        );
        for w in pairs.windows(2) {
            assert!(w[0].0 < w[1].0 && w[0].1 < w[1].1);
        }
        assert!(pairs.len() >= 2);
    }

    #[test]
    fn threshold_gates_weak_matches() {
        let old = ["completely unrelated content alpha"];
        let new = ["nothing shared whatsoever beta"];
        let sim = setup(&old, &new);
        let ob: Vec<Block> = old.iter().map(|t| paragraph(t)).collect();
        let nb: Vec<Block> = new.iter().map(|t| paragraph(t)).collect();

        // Unreachable for these texts.
        let cfg = AlignConfig {
            similarity_threshold: 0.95,
            ..AlignConfig::default()
        };
        let pairs = align_free_blocks(&ob, &nb, &[0], &[0], &sim, &cfg);
        assert!(pairs.is_empty());
    }

    #[test]
    fn chunking_never_panics_and_aligns_within_windows() {
        let texts: Vec<String> = (0..10).map(|k| format!("document body text {k}")).collect();
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let old_texts: Vec<&str> = refs.clone();
        let new_texts: Vec<&str> = refs.clone();
        let sim = setup(&old_texts, &new_texts);
        let ob: Vec<Block> = refs.iter().map(|t| paragraph(t)).collect();
        let nb: Vec<Block> = refs.iter().map(|t| paragraph(t)).collect();

        // Force many windows.
        let cfg = AlignConfig {
            max_chunk: 3,
            ..AlignConfig::default()
        };
        let pairs = align_free_blocks(
            &ob,
            &nb,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            &sim,
            &cfg,
        );
        // Identical layouts: every window pair matches fully.
        assert_eq!(pairs.len(), 10);
    }

    #[test]
    fn headings_gap_cost_is_heavier() {
        let cfg = AlignConfig::default();
        let heading = Block {
            id: String::new(),
            kind: BlockKind::Heading { level: 1 },
            text: "H".into(),
            page: None,
            rect: None,
            anchor: None,
        };
        let para = paragraph("P");
        assert!(gap_cost(&heading, &cfg) < gap_cost(&para, &cfg));
        let cfg_flat = AlignConfig {
            heading_gap_weight: 1.0,
            ..AlignConfig::default()
        };
        assert_eq!(gap_cost(&heading, &cfg_flat), gap_cost(&para, &cfg_flat));
    }
}
