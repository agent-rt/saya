//! Hybrid search.
//!
//! Combines a literal/BM25 lane (Tantivy + jieba-rs tokenizer) with a vector
//! lane (sqlite-vec cosine over MiniLM-L6-v2 embeddings) by Reciprocal Rank
//! Fusion. When no embedder is attached, the vector lane is skipped and
//! results match the BM25 lane verbatim.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Database;

#[cfg(feature = "embedding")]
use crate::ai::EmbedderHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub id: i64,
    pub content: String,
    pub score: f32,
    pub created_at: i64,
}

/// RRF k constant. Standard literature uses 60.
const RRF_K: f32 = 60.0;

pub struct Searcher {
    db: Database,
    #[cfg(feature = "embedding")]
    embedder: Option<EmbedderHandle>,
}

impl Searcher {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            #[cfg(feature = "embedding")]
            embedder: None,
        }
    }

    #[cfg(feature = "embedding")]
    pub fn with_embedder(mut self, embedder: EmbedderHandle) -> Self {
        self.embedder = Some(embedder);
        self
    }

    pub fn search(&self, query: &SearchQuery) -> crate::Result<Vec<SearchHit>> {
        let limit = query.limit.max(1);
        // Over-fetch each lane so RRF has room to re-rank.
        let lane_limit = (limit * 4).max(20);

        let bm25_ranking: Vec<i64> = self
            .db
            .bm25_search(&query.text, lane_limit)?
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        #[cfg_attr(not(feature = "embedding"), allow(unused_mut))]
        let mut rankings: Vec<Vec<i64>> = vec![bm25_ranking];

        #[cfg(feature = "embedding")]
        if let Some(emb) = &self.embedder {
            match emb.embed_one(&query.text) {
                Ok(qv) => match self.db.vector_search(&qv, lane_limit) {
                    Ok(hits) => {
                        rankings.push(hits.into_iter().map(|(id, _)| id).collect());
                    }
                    Err(e) => tracing::warn!(error = %e, "vector_search failed"),
                },
                Err(e) => tracing::warn!(error = %e, "query embed failed; using literal-only"),
            }
        }

        let merged = rrf(&rankings, RRF_K);

        let mut hits = Vec::with_capacity(limit);
        for (id, score) in merged.into_iter().take(limit) {
            if let Some(e) = self.db.get_entry(id)? {
                hits.push(SearchHit {
                    id: e.id,
                    content: e.content,
                    score,
                    created_at: e.created_at,
                });
            }
        }
        Ok(hits)
    }
}

/// Convenience wrapper for callers that don't need an embedder.
pub fn search(db: &Database, query: &SearchQuery) -> crate::Result<Vec<SearchHit>> {
    Searcher::new(db.clone()).search(query)
}

/// Reciprocal Rank Fusion. Each ranking is a list of doc ids in descending
/// relevance. Returns docs sorted by combined score (higher is better).
fn rrf(rankings: &[Vec<i64>], k: f32) -> Vec<(i64, f32)> {
    let mut scores: HashMap<i64, f32> = HashMap::new();
    for ranking in rankings {
        for (idx, &id) in ranking.iter().enumerate() {
            let rank = (idx + 1) as f32;
            *scores.entry(id).or_insert(0.0) += 1.0 / (k + rank);
        }
    }
    let mut v: Vec<(i64, f32)> = scores.into_iter().collect();
    v.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_merges_two_lanes() {
        let a = vec![1, 2, 3];
        let b = vec![3, 4, 1];
        let merged = rrf(&[a, b], 60.0);
        // 1 and 3 appear in both lanes -> should outrank single-lane 2 and 4.
        let ids: Vec<i64> = merged.iter().map(|(id, _)| *id).collect();
        let pos_1 = ids.iter().position(|&x| x == 1).unwrap();
        let pos_3 = ids.iter().position(|&x| x == 3).unwrap();
        let pos_2 = ids.iter().position(|&x| x == 2).unwrap();
        let pos_4 = ids.iter().position(|&x| x == 4).unwrap();
        assert!(pos_1 < pos_2);
        assert!(pos_3 < pos_4);
    }

    #[test]
    fn literal_only_when_no_embedder() {
        let db = Database::open_in_memory().unwrap();
        db.insert_entry("the quick brown fox").unwrap();
        db.insert_entry("hello world").unwrap();
        let hits = Searcher::new(db)
            .search(&SearchQuery { text: "brown".into(), limit: 10 })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("brown"));
    }
}
