//! Real S3 backend for `tdw_core::BlobEngine`.
//!
//! Gated by the `s3` feature. Wraps [`aws_sdk_s3::Client`] and dispatches
//! `put_object` / `get_object` to S3 (or any S3-compatible endpoint
//! including `MinIO`, R2, Backblaze B2, etc. via `S3Engine::from_endpoint`).
//!
//! The same key-validation rules as [`crate::InMemoryS3BlobEngine`]
//! apply: keys must be non-empty, must not contain `\\`, and must be
//! relative + canonical (no `..`, `./`, leading `/`).

use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use tdw_core::{BlobEngine, Error, Result};

use crate::validate_key;

/// Production S3 backend.
///
/// Construct via [`S3Engine::from_env`] for AWS
/// production credentials (taken from the standard AWS env vars /
/// credentials file), or [`S3Engine::from_endpoint`] for `MinIO` /
/// other S3-compatible services with explicit endpoint + credentials.
#[derive(Clone, Debug)]
pub struct S3Engine {
    client: Client,
    bucket: String,
}

impl S3Engine {
    /// Construct from the ambient AWS environment (env vars, shared
    /// config files, instance profile, etc.) — same resolution chain
    /// the AWS CLI uses.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn from_env(bucket: impl Into<String>) -> Result<Self> {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Ok(Self {
            client: Client::new(&config),
            bucket: bucket.into(),
        })
    }

    /// Construct against an explicit endpoint (typical for `MinIO` / R2 /
    /// any S3-compatible service). Uses path-style addressing because
    /// `MinIO` and most non-AWS services need it.
    pub fn from_endpoint(
        endpoint_url: impl Into<String>,
        region: impl Into<String>,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        bucket: impl Into<String>,
    ) -> Self {
        let credentials = Credentials::new(
            access_key_id.into(),
            secret_access_key.into(),
            None,
            None,
            "tdw-storage-s3",
        );
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .endpoint_url(endpoint_url.into())
            .region(Region::new(region.into()))
            .credentials_provider(credentials)
            .force_path_style(true)
            .build();
        Self {
            client: Client::from_conf(config),
            bucket: bucket.into(),
        }
    }

    /// Adopt a caller-built client (useful for test harnesses that
    /// already configured a client).
    pub fn with_client(client: Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
        }
    }

    /// Expose the bucket name this engine writes to.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }
}

#[async_trait]
impl BlobEngine for S3Engine {
    async fn put_object(&self, key: &str, body: Bytes, content_type: &str) -> Result<()> {
        validate_key(key)?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(body))
            .content_type(content_type)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("s3 put_object: {error}")))?;
        Ok(())
    }

    async fn get_object(&self, key: &str) -> Result<Bytes> {
        validate_key(key)?;
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("s3 get_object: {error}")))?;
        let aggregated = output
            .body
            .collect()
            .await
            .map_err(|error| Error::Storage(format!("s3 read body: {error}")))?;
        Ok(aggregated.into_bytes())
    }
}
