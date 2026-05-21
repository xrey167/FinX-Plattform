#![forbid(unsafe_code)]

use std::collections::BTreeMap;
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
        self.objects
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?
            .get(key)
            .map(|object| object.body.clone())
            .ok_or_else(|| Error::Storage(format!("missing s3 object: {key}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_blob_engine_contract() {
        fn assert_blob<T: BlobEngine>() {}

        assert_blob::<InMemoryS3BlobEngine>();
    }
}
