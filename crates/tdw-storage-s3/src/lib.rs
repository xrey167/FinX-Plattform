#![forbid(unsafe_code)]

#[cfg(feature = "s3")]
pub mod aws_engine;

#[cfg(feature = "s3")]
pub use aws_engine::S3Engine;

use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use tdw_core::{BlobEngine, Error, Result};

#[derive(Debug, Default)]
pub struct InMemoryS3BlobEngine {
    objects: Mutex<BTreeMap<String, StoredObject>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObject {
    pub body: Bytes,
    pub content_type: String,
}

impl InMemoryS3BlobEngine {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn object_count(&self) -> Result<usize> {
        self.objects
            .lock()
            .map(|objects| objects.len())
            .map_err(|error| Error::Storage(error.to_string()))
    }
}

#[async_trait]
impl BlobEngine for InMemoryS3BlobEngine {
    async fn put_object(&self, key: &str, body: Bytes, content_type: &str) -> Result<()> {
        validate_key(key)?;
        self.objects
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?
            .insert(
                key.to_string(),
                StoredObject {
                    body,
                    content_type: content_type.to_string(),
                },
            );
        Ok(())
    }

    async fn get_object(&self, key: &str) -> Result<Bytes> {
        validate_key(key)?;
        self.objects
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?
            .get(key)
            .map(|object| object.body.clone())
            .ok_or_else(|| Error::Storage(format!("missing s3 object: {key}")))
    }
}

pub(crate) fn validate_key(key: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(Error::Storage(
            "s3 object key must not be empty".to_string(),
        ));
    }
    if key.contains('\\') {
        return Err(Error::Storage(format!(
            "s3 object key must use '/' separators: {key}"
        )));
    }
    for component in Path::new(key).components() {
        match component {
            Component::Normal(part) if !part.is_empty() => {}
            _ => {
                return Err(Error::Storage(format!(
                    "s3 object key must be relative and canonical: {key}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_blob_engine_contract() {
        fn assert_blob<T: BlobEngine>() {}

        assert_blob::<InMemoryS3BlobEngine>();
    }

    #[tokio::test]
    async fn stores_objects_under_canonical_relative_keys() {
        let engine = InMemoryS3BlobEngine::default();
        engine
            .put_object(
                "raw/ohlcv.parquet",
                Bytes::from_static(b"parquet"),
                "application/vnd.apache.parquet",
            )
            .await
            .unwrap_or_else(|error| panic!("object should store: {error}"));

        let body = engine
            .get_object("raw/ohlcv.parquet")
            .await
            .unwrap_or_else(|error| panic!("object should read: {error}"));

        assert_eq!(body, Bytes::from_static(b"parquet"));
    }

    #[tokio::test]
    async fn rejects_absolute_parent_and_platform_keys() {
        let engine = InMemoryS3BlobEngine::default();

        for key in [
            "",
            "../secret.parquet",
            "/rooted.parquet",
            "raw\\ohlcv.parquet",
        ] {
            assert!(
                engine
                    .put_object(key, Bytes::from_static(b"bad"), "application/octet-stream")
                    .await
                    .is_err()
            );
        }
    }
}
