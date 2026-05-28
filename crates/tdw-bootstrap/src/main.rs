//! `tdw-bootstrap` - one-shot binary that brings the data backend live.
//!
//! Reads connection details from environment variables, applies all
//! Postgres schemas needed by the G013 durable-persistence crates,
//! and writes a marker object to the configured S3/MinIO bucket so
//! downstream services can confirm the bucket is reachable.
//!
//! Emits one structured JSON line per step on stdout, suitable for
//! `docker compose logs tdw-bootstrap` and CI grep.
//!
//! Required env vars:
//!   - `TDW_POSTGRES_URL`     e.g. `postgres://tdw:tdw@postgres:5432/tdw`
//!   - `TDW_S3_ENDPOINT`      e.g. `http://minio:9000`
//!   - `TDW_S3_BUCKET`        e.g. `tdw-default`
//!   - `TDW_S3_ACCESS_KEY`    e.g. `minio`
//!   - `TDW_S3_SECRET_KEY`    e.g. `minio123`
//!
//! Optional:
//!   - `TDW_S3_REGION`        default `us-east-1`
//!
//! Exits 0 on full success, non-zero on the first failed step.

#![forbid(unsafe_code)]

use std::env;
use std::process::ExitCode;

use bytes::Bytes;
use serde_json::json;
use tdw_bus::PgEventBus;
use tdw_core::BlobEngine;
use tdw_outbox::PgOutboxStore;
use tdw_session::PgSessionStore;
use tdw_snapshot::PgSnapshotStore;
use tdw_storage_postgres::PgEngine;
use tdw_storage_s3::S3Engine;

const MARKER_KEY: &str = "_tdw_bootstrap_marker";
const MARKER_BODY: &[u8] = b"tdw-bootstrap ok\n";

#[tokio::main]
async fn main() -> ExitCode {
    log_step("start", "starting", None);

    let postgres_url = match require_env("TDW_POSTGRES_URL") {
        Ok(value) => value,
        Err(error) => {
            log_step("env", "failed", Some(&error));
            return ExitCode::from(2);
        }
    };

    let s3_endpoint = match require_env("TDW_S3_ENDPOINT") {
        Ok(value) => value,
        Err(error) => {
            log_step("env", "failed", Some(&error));
            return ExitCode::from(2);
        }
    };
    let s3_bucket = match require_env("TDW_S3_BUCKET") {
        Ok(value) => value,
        Err(error) => {
            log_step("env", "failed", Some(&error));
            return ExitCode::from(2);
        }
    };
    let s3_access_key = match require_env("TDW_S3_ACCESS_KEY") {
        Ok(value) => value,
        Err(error) => {
            log_step("env", "failed", Some(&error));
            return ExitCode::from(2);
        }
    };
    let s3_secret_key = match require_env("TDW_S3_SECRET_KEY") {
        Ok(value) => value,
        Err(error) => {
            log_step("env", "failed", Some(&error));
            return ExitCode::from(2);
        }
    };
    let s3_region = env::var("TDW_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());

    log_step("env", "ok", Some("required env vars present"));

    // Postgres connect + schemas.
    let engine = match PgEngine::connect(&postgres_url).await {
        Ok(engine) => engine,
        Err(error) => {
            log_step("postgres-connect", "failed", Some(&error.to_string()));
            return ExitCode::from(3);
        }
    };
    log_step("postgres-connect", "ok", None);

    if let Err(error) = PgOutboxStore::new(engine.clone()).ensure_schema().await {
        log_step("outbox-schema", "failed", Some(&error.to_string()));
        return ExitCode::from(4);
    }
    log_step("outbox-schema", "ok", Some("tdw_outbox"));

    if let Err(error) = PgSnapshotStore::new(engine.clone()).ensure_schema().await {
        log_step("snapshot-schema", "failed", Some(&error.to_string()));
        return ExitCode::from(4);
    }
    log_step("snapshot-schema", "ok", Some("tdw_snapshot"));

    if let Err(error) = PgEventBus::new(engine.clone()).ensure_schema().await {
        log_step("bus-schema", "failed", Some(&error.to_string()));
        return ExitCode::from(4);
    }
    log_step("bus-schema", "ok", Some("tdw_bus"));

    if let Err(error) = PgSessionStore::new(engine.clone()).ensure_schema().await {
        log_step("session-schema", "failed", Some(&error.to_string()));
        return ExitCode::from(4);
    }
    log_step(
        "session-schema",
        "ok",
        Some(
            "tdw_sessions + tdw_sessions_permission_state + tdw_sessions_pending_approvals + tdw_sessions_cost_ledger",
        ),
    );

    // S3 / MinIO marker write.
    let s3 = S3Engine::from_endpoint(
        &s3_endpoint,
        &s3_region,
        &s3_access_key,
        &s3_secret_key,
        &s3_bucket,
    );

    let body = Bytes::from_static(MARKER_BODY);
    if let Err(error) = s3.put_object(MARKER_KEY, body.clone(), "text/plain").await {
        log_step(
            "s3-marker",
            "failed",
            Some(&format!(
                "write {MARKER_KEY} to bucket {s3_bucket}: {error} \
                 (hint: ensure the bucket exists; minio-init compose service creates it)"
            )),
        );
        return ExitCode::from(5);
    }
    log_step(
        "s3-marker",
        "ok",
        Some(&format!("wrote {MARKER_KEY} to {s3_bucket}")),
    );

    // Verify roundtrip.
    match s3.get_object(MARKER_KEY).await {
        Ok(read_back) if read_back == body => {
            log_step("s3-roundtrip", "ok", Some("marker bytes match"));
        }
        Ok(_) => {
            log_step(
                "s3-roundtrip",
                "failed",
                Some("marker read-back bytes did not match written body"),
            );
            return ExitCode::from(6);
        }
        Err(error) => {
            log_step("s3-roundtrip", "failed", Some(&error.to_string()));
            return ExitCode::from(6);
        }
    }

    log_step("done", "ok", Some("data backend live"));
    ExitCode::SUCCESS
}

fn require_env(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("env var {name} is required"))
}

fn log_step(step: &str, status: &str, detail: Option<&str>) {
    let mut record = json!({
        "step": step,
        "status": status,
    });
    if let Some(detail) = detail {
        record["detail"] = json!(detail);
    }
    println!("{record}");
}
