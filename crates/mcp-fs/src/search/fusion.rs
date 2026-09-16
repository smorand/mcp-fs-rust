//! Reciprocal Rank Fusion (RRF) for merging BM25 and vector result lists.
//!
//! RRF with k=60 is the standard choice for hybrid retrieval: it is parameter
//! insensitive over a wide range of corpus sizes and consistently outperforms
//! simple score combination without requiring calibration.
//!
//! The merge is deterministic: when two entries have the same fused score they
//! are ordered by path (lexicographic), so the output is stable across calls.

use crate::search::SearchResult;
use std::collections::HashMap;

/// Standard RRF constant. Higher values dampen the influence of top-ranked items.
const K: f32 = 60.0;

/// Merge two ranked result lists using Reciprocal Rank Fusion.
///
/// A document appearing in both lists scores higher than one in only one list.
/// The returned list is re-ranked and rank indices are assigned from 1.
pub fn rrf_merge(bm25: &[SearchResult], vector: &[SearchResult]) -> Vec<SearchResult> {
    // Map path -> cumulative RRF score, best chunk from either list.
    let mut scores: HashMap<String, f32> = HashMap::new();
    let mut best: HashMap<String, (String, f32)> = HashMap::new(); // path -> (chunk, score)

    for (i, r) in bm25.iter().enumerate() {
        let rrf = 1.0 / (K + (i as f32) + 1.0);
        *scores.entry(r.path.clone()).or_default() += rrf;
        best.entry(r.path.clone())
            .and_modify(|(c, s)| {
                if r.score > *s {
                    *c = r.chunk.clone();
                    *s = r.score;
                }
            })
            .or_insert_with(|| (r.chunk.clone(), r.score));
    }

    for (i, r) in vector.iter().enumerate() {
        let rrf = 1.0 / (K + (i as f32) + 1.0);
        *scores.entry(r.path.clone()).or_default() += rrf;
        best.entry(r.path.clone())
            .and_modify(|(c, s)| {
                if r.score > *s {
                    *c = r.chunk.clone();
                    *s = r.score;
                }
            })
            .or_insert_with(|| (r.chunk.clone(), r.score));
    }

    // Sort by fused score descending, then by path for determinism.
    let mut entries: Vec<(String, f32)> = scores.into_iter().collect();
    entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));

    entries
        .into_iter()
        .enumerate()
        .map(|(i, (path, fused_score))| {
            let (chunk, _orig_score) = best.remove(&path).unwrap_or_default();
            SearchResult { path, score: fused_score, chunk, rank: i + 1 }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(path: &str, score: f32, chunk: &str, rank: usize) -> SearchResult {
        SearchResult { path: path.into(), score, chunk: chunk.into(), rank }
    }

    #[test]
    fn rrf_merge_is_deterministic() {
        let bm25 = vec![r("/a.md", 0.9, "chunk a", 1), r("/b.md", 0.5, "chunk b", 2)];
        let vec_ = vec![r("/b.md", 0.8, "chunk b", 1), r("/a.md", 0.4, "chunk a", 2)];
        let out1 = rrf_merge(&bm25, &vec_);
        let out2 = rrf_merge(&bm25, &vec_);
        assert_eq!(
            out1.iter().map(|r| &r.path).collect::<Vec<_>>(),
            out2.iter().map(|r| &r.path).collect::<Vec<_>>(),
            "same inputs must produce the same order"
        );
    }

    #[test]
    fn rrf_merge_prefers_appearing_in_both_lists() {
        // /shared.md appears in both lists at rank 2; /only_bm25.md only in bm25 at rank 1.
        let bm25 = vec![
            r("/only_bm25.md", 1.0, "x", 1),
            r("/shared.md", 0.5, "y", 2),
        ];
        let vec_ = vec![
            r("/only_vec.md", 1.0, "z", 1),
            r("/shared.md", 0.5, "y", 2),
        ];
        let out = rrf_merge(&bm25, &vec_);
        let paths: Vec<&str> = out.iter().map(|r| r.path.as_str()).collect();
        let shared_pos = paths.iter().position(|p| *p == "/shared.md").unwrap();
        let only_bm25_pos = paths.iter().position(|p| *p == "/only_bm25.md").unwrap();
        assert!(
            shared_pos < only_bm25_pos,
            "a doc in both lists should rank above one in only one list: {paths:?}"
        );
    }

    #[test]
    fn empty_inputs_produce_empty_output() {
        assert!(rrf_merge(&[], &[]).is_empty());
    }

    #[test]
    fn ranks_are_assigned_starting_at_one() {
        let bm25 = vec![r("/a.md", 1.0, "x", 1)];
        let out = rrf_merge(&bm25, &[]);
        assert_eq!(out[0].rank, 1);
    }
}
