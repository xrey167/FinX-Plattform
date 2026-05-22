#![forbid(unsafe_code)]

use std::fs;
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;
use tdw_core::{BlobEngine, Error, Result};

#[derive(Clone, Debug)]
pub struct LocalBlobEngine {
    root: PathBuf,
}

impl LocalBlobEngine {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve_key(&self, key: &str) -> Result<PathBuf> {
        if key.trim().is_empty() || key.contains('\\') {
            return Err(Error::Storage(format!("invalid blob key: {key}")));
        }
        let path = Path::new(key);
        if path.components().any(|component| {
            matches!(
                component,
                Component::CurDir
                    | Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        }) {
            return Err(Error::Storage(format!("invalid blob key: {key}")));
        }
        Ok(self.root.join(path))
    }
}

#[async_trait]
impl BlobEngine for LocalBlobEngine {
    async fn put_object(&self, key: &str, body: Bytes, _content_type: &str) -> Result<()> {
        let path = self.resolve_key(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| Error::Storage(error.to_string()))?;
        }
        fs::write(path, body).map_err(|error| Error::Storage(error.to_string()))
    }

    async fn get_object(&self, key: &str) -> Result<Bytes> {
        let path = self.resolve_key(key)?;
        let body = fs::read(path).map_err(|error| Error::Storage(error.to_string()))?;
        Ok(Bytes::from(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal_keys() {
        let engine = LocalBlobEngine::new("target/blob-tests");

        assert!(engine.resolve_key("../escape").is_err());
        assert!(engine.resolve_key("").is_err());
        assert!(engine.resolve_key("./object.json").is_err());
        assert!(engine.resolve_key("safe\\object.json").is_err());
        assert!(engine.resolve_key("safe/object.json").is_ok());
    }

    #[tokio::test]
    async fn writes_and_reads_relative_keys() {
        let root = std::env::temp_dir().join(format!("tdw-storage-fs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let engine = LocalBlobEngine::new(&root);
        engine
            .put_object(
                "safe/object.bin",
                Bytes::from_static(b"payload"),
                "application/octet-stream",
            )
            .await
            .unwrap_or_else(|error| panic!("object should write: {error}"));

        let body = engine
            .get_object("safe/object.bin")
            .await
            .unwrap_or_else(|error| panic!("object should read: {error}"));

        assert_eq!(body, Bytes::from_static(b"payload"));
        let _ = fs::remove_dir_all(root);
    }
}
