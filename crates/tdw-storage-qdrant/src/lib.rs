#![forbid(unsafe_code)]

#[cfg(feature = "qdrant")]
pub mod http_engine;

#[cfg(feature = "qdrant")]
pub use http_engine::QdrantHttpEngine;

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
        validate_collection(collection)?;
        if points.is_empty() {
            return Err(Error::Storage(
                "vector upsert must include at least one point".to_string(),
            ));
        }
        let expected_dimension = points[0].vector.len();
        for point in &points {
            validate_point(point)?;
            if point.vector.len() != expected_dimension {
                return Err(Error::Storage(format!(
                    "vector dimension mismatch for point {}: expected={}, actual={}",
                    point.id,
                    expected_dimension,
                    point.vector.len()
                )));
            }
        }
        let mut collections = self
            .collections
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        let collection_points = collections.entry(collection.to_string()).or_default();
        if let Some(existing) = collection_points.first()
            && existing.vector.len() != expected_dimension
        {
            return Err(Error::Storage(format!(
                "vector collection {collection} dimension is {}, got {}",
                existing.vector.len(),
                expected_dimension
            )));
        }
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
        drop(collections);
        Ok(())
    }

    async fn search_knn(&self, collection: &str, query: VectorQuery) -> Result<Vec<ScoredPoint>> {
        validate_collection(collection)?;
        if query.vector.is_empty() {
            return Err(Error::Storage("query vector must not be empty".to_string()));
        }
        if query.top_k == 0 {
            return Err(Error::Storage(
                "query top_k must be greater than zero".to_string(),
            ));
        }
        let mut scored = {
            let collections = self
                .collections
                .lock()
                .map_err(|error| Error::Storage(error.to_string()))?;
            let points = collections.get(collection).ok_or_else(|| {
                Error::Storage(format!("unknown vector collection: {collection}"))
            })?;
            for point in points {
                if point.vector.len() != query.vector.len() {
                    return Err(Error::Storage(format!(
                        "vector dimension mismatch for point {}: query={}, point={}",
                        point.id,
                        query.vector.len(),
                        point.vector.len()
                    )));
                }
            }
            let collected = points
                .iter()
                .filter(|point| query.filter.matches(&point.payload))
                .map(|point| ScoredPoint {
                    id: point.id.clone(),
                    score: dot_product(&point.vector, &query.vector),
                    payload: point.payload.clone(),
                })
                .collect::<Vec<_>>();
            drop(collections);
            collected
        };

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

fn validate_collection(collection: &str) -> Result<()> {
    if collection.trim().is_empty() {
        return Err(Error::Storage(
            "vector collection name must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_point(point: &VectorPoint) -> Result<()> {
    if point.id.trim().is_empty() {
        return Err(Error::Storage(
            "vector point id must not be empty".to_string(),
        ));
    }
    if point.vector.is_empty() {
        return Err(Error::Storage(format!(
            "vector point {} must not have an empty vector",
            point.id
        )));
    }
    Ok(())
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

    #[tokio::test]
    async fn searches_by_score_and_rejects_invalid_dimensions() {
        let engine = InMemoryVectorEngine::default();
        engine
            .upsert(
                "research",
                vec![
                    VectorPoint {
                        id: "a".to_string(),
                        vector: vec![1.0, 0.0],
                        payload: serde_json::json!({ "rank": "first" }),
                    },
                    VectorPoint {
                        id: "b".to_string(),
                        vector: vec![0.25, 1.0],
                        payload: serde_json::json!({ "rank": "second" }),
                    },
                ],
            )
            .await
            .unwrap_or_else(|error| panic!("points should upsert: {error}"));

        let scored = engine
            .search_knn(
                "research",
                VectorQuery {
                    vector: vec![1.0, 0.0],
                    top_k: 1,
                    filter: tdw_core::PayloadFilter::default(),
                },
            )
            .await
            .unwrap_or_else(|error| panic!("search should score matching dimensions: {error}"));

        assert_eq!(scored.len(), 1);
        assert_eq!(scored[0].id, "a");
        assert!(
            engine
                .search_knn(
                    "research",
                    VectorQuery {
                        vector: vec![1.0],
                        top_k: 1,
                        filter: tdw_core::PayloadFilter::default(),
                    },
                )
                .await
                .is_err()
        );
        assert!(engine.upsert("research", Vec::new()).await.is_err());
        assert!(
            engine
                .search_knn(
                    "research",
                    VectorQuery {
                        vector: vec![1.0, 0.0],
                        top_k: 0,
                        filter: tdw_core::PayloadFilter::default(),
                    },
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn payload_filter_excludes_points_before_scoring() {
        use tdw_core::{PayloadCondition, PayloadFilter};
        let engine = InMemoryVectorEngine::default();
        engine
            .upsert(
                "research",
                vec![
                    VectorPoint {
                        id: "equity-doc".to_string(),
                        vector: vec![1.0, 0.0],
                        payload: serde_json::json!({
                            "tags": ["asset:equity"],
                            "as_of": "2026-01-01T00:00:00Z",
                        }),
                    },
                    VectorPoint {
                        id: "crypto-doc".to_string(),
                        vector: vec![0.99, 0.1],
                        payload: serde_json::json!({
                            "tags": ["asset:crypto"],
                            "as_of": "2026-06-01T00:00:00Z",
                        }),
                    },
                ],
            )
            .await
            .unwrap_or_else(|error| panic!("points should upsert: {error}"));

        // Tag filter: the otherwise-best crypto hit is excluded entirely.
        let filtered = engine
            .search_knn(
                "research",
                VectorQuery {
                    vector: vec![1.0, 0.0],
                    top_k: 2,
                    filter: PayloadFilter {
                        must: vec![PayloadCondition::MatchAny {
                            key: "tags".to_string(),
                            values: vec!["asset:equity".to_string()],
                        }],
                    },
                },
            )
            .await
            .unwrap_or_else(|error| panic!("filtered search should succeed: {error}"));
        assert_eq!(
            filtered
                .iter()
                .map(|hit| hit.id.as_str())
                .collect::<Vec<_>>(),
            vec!["equity-doc"]
        );

        // as_of range filter hides later documents (leakage safety).
        let as_of = engine
            .search_knn(
                "research",
                VectorQuery {
                    vector: vec![1.0, 0.0],
                    top_k: 2,
                    filter: PayloadFilter {
                        must: vec![PayloadCondition::RangeString {
                            key: "as_of".to_string(),
                            gte: None,
                            lte: Some("2026-03-01T00:00:00Z".to_string()),
                        }],
                    },
                },
            )
            .await
            .unwrap_or_else(|error| panic!("as_of search should succeed: {error}"));
        assert_eq!(
            as_of.iter().map(|hit| hit.id.as_str()).collect::<Vec<_>>(),
            vec!["equity-doc"]
        );
    }
}
