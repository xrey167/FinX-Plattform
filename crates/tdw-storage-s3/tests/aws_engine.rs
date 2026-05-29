//! Integration test for the real S3 backend.
//!
//! Gated two ways:
//!   - Compiled only with `--features s3` (no aws-sdk-s3 dep otherwise).
//!   - Runs only when `TDW_S3_TEST_BUCKET` and `TDW_S3_TEST_ENDPOINT`
//!     are both set in the environment. CI workflows that bring up a
//!     `MinIO` docker container should set these; default
//!     `cargo test --workspace` leaves them unset and the test silently
//!     skips.
//!
//! Required env vars:
//!   - `TDW_S3_TEST_BUCKET`   — bucket name (must already exist)
//!   - `TDW_S3_TEST_ENDPOINT` — base URL (e.g. <http://127.0.0.1:9000>)
//!
//! Optional env vars (default to `MinIO`'s standard root credentials):
//!   - `TDW_S3_TEST_ACCESS_KEY` (default `minioadmin`)
//!   - `TDW_S3_TEST_SECRET_KEY` (default `minioadmin`)
//!   - `TDW_S3_TEST_REGION`     (default `us-east-1`)

#![cfg(feature = "s3")]

use bytes::Bytes;
use tdw_core::BlobEngine;
use tdw_storage_s3::S3Engine;

struct TestConfig {
    bucket: String,
    endpoint: String,
    access_key: String,
    secret_key: String,
    region: String,
}

fn config() -> Option<TestConfig> {
    let bucket = std::env::var("TDW_S3_TEST_BUCKET").ok()?;
    let endpoint = std::env::var("TDW_S3_TEST_ENDPOINT").ok()?;
    Some(TestConfig {
        bucket,
        endpoint,
        access_key: std::env::var("TDW_S3_TEST_ACCESS_KEY")
            .unwrap_or_else(|_| "minioadmin".to_string()),
        secret_key: std::env::var("TDW_S3_TEST_SECRET_KEY")
            .unwrap_or_else(|_| "minioadmin".to_string()),
        region: std::env::var("TDW_S3_TEST_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
    })
}

#[tokio::test]
async fn s3_engine_round_trips_object_against_real_s3() {
    let Some(cfg) = config() else {
        eprintln!(
            "TDW_S3_TEST_BUCKET / TDW_S3_TEST_ENDPOINT not set; skipping S3 integration test"
        );
        return;
    };

    let engine = S3Engine::from_endpoint(
        &cfg.endpoint,
        &cfg.region,
        &cfg.access_key,
        &cfg.secret_key,
        &cfg.bucket,
    );

    let key = format!("tdw-g010-smoke/{}.bin", std::process::id());
    let payload = Bytes::from_static(b"tdw-g010-s3-smoke payload");

    engine
        .put_object(&key, payload.clone(), "application/octet-stream")
        .await
        .unwrap_or_else(|error| panic!("put_object must succeed: {error}"));

    let roundtrip = engine
        .get_object(&key)
        .await
        .unwrap_or_else(|error| panic!("get_object must succeed: {error}"));

    assert_eq!(roundtrip, payload, "S3 must return byte-exact roundtrip");
}

#[tokio::test]
async fn s3_engine_rejects_invalid_keys() {
    let Some(cfg) = config() else {
        return;
    };

    let engine = S3Engine::from_endpoint(
        &cfg.endpoint,
        &cfg.region,
        &cfg.access_key,
        &cfg.secret_key,
        &cfg.bucket,
    );

    let err = engine
        .put_object("", Bytes::from_static(b"x"), "application/octet-stream")
        .await
        .expect_err("empty key must be rejected");
    assert!(err.to_string().contains("must not be empty"));

    let err = engine
        .put_object(
            "..\\escape",
            Bytes::from_static(b"x"),
            "application/octet-stream",
        )
        .await
        .expect_err("backslash key must be rejected");
    assert!(err.to_string().contains("'/' separators"));
}
