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
        let path = Path::new(key);
        if path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
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
        assert!(engine.resolve_key("safe/object.json").is_ok());
    }
}
