#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use tdw_core::{Error, Result, ScoredPoint, VectorEngine, VectorPoint, VectorQuery};

#[derive(Debug, Default)]
pub struct InMemoryVectorEngine {
    collections: Mutex<BTreeMap<String, Vec<VectorPoint>>>,
}

#[async_trait]
impl VectorEngine for InMemoryVectorEngine {
    async fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<()> {
        let mut collections = self
            .collections
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        let collection_points = collections.entry(collection.to_string()).or_default();
        for point in points {
            if let Some(existing) = collection_points
                .iter_mut()
                .find(|existing| existing.id == point.id)
            {
                *existing = point;
            } else {
                collection_points.push(point);
            }
        }
        Ok(())
    }

    async fn search_knn(&self, collection: &str, query: VectorQuery) -> Result<Vec<ScoredPoint>> {
        let collections = self
            .collections
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        let points = collections
            .get(collection)
            .ok_or_else(|| Error::Storage(format!("unknown vector collection: {collection}")))?;
        let mut scored = points
            .iter()
            .map(|point| ScoredPoint {
                id: point.id.clone(),
                score: dot_product(&point.vector, &query.vector),
                payload: point.payload.clone(),
            })
            .collect::<Vec<_>>();

        scored.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
        });
        scored.truncate(query.top_k);
        Ok(scored)
    }
}

fn dot_product(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_vector_engine_contract() {
        fn assert_vector<T: VectorEngine>() {}

        assert_vector::<InMemoryVectorEngine>();
    }
}
