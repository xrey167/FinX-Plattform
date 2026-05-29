//! `tdw-bootstrap` - one-shot binary that brings the data backend live.
//!
//! Reads connection details from environment variables, applies all
//! Postgres schemas needed by the G013 durable-persistence crates,
//! writes a marker object to the configured S3/MinIO bucket, and -
//! when their endpoints are configured - creates a baseline schema in
//! `ClickHouse`, Qdrant, and Meilisearch so the full search/OLAP/vector
//! backend is reachable before any application service starts.
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
//!   - `TDW_CLICKHOUSE_URL`   e.g. `http://clickhouse:8123` (+ `_USER`/`_PASSWORD`)
//!   - `TDW_QDRANT_URL`       e.g. `http://qdrant:6333` (+ `_API_KEY`, `_VECTOR_SIZE`)
//!   - `TDW_MEILI_URL`        e.g. `http://meilisearch:7700` (+ `_API_KEY`)
//!
//! Each optional backend is skipped unless its `*_URL` is set, so the
//! minimal Postgres + S3 bootstrap keeps working unchanged.
//!
//! Exits 0 on full success, non-zero on the first failed step. Exit codes:
//! 2 env, 3 postgres-connect, 4 postgres-schema, 5 s3-marker, 6 s3-roundtrip,
//! 7 clickhouse, 8 qdrant, 9 meilisearch.

#![forbid(unsafe_code)]

use std::env;
use std::process::ExitCode;

use bytes::Bytes;
use serde_json::json;
use tdw_bus::PgEventBus;
use tdw_core::{BlobEngine, OlapEngine};
use tdw_outbox::PgOutboxStore;
use tdw_session::PgSessionStore;
use tdw_snapshot::PgSnapshotStore;
use tdw_storage_clickhouse::ClickHouseHttpEngine;
use tdw_storage_meilisearch::MeilisearchHttpEngine;
use tdw_storage_postgres::PgEngine;
use tdw_storage_qdrant::QdrantHttpEngine;
use tdw_storage_s3::S3Engine;

const MARKER_KEY: &str = "_tdw_bootstrap_marker";
const MARKER_BODY: &[u8] = b"tdw-bootstrap ok\n";
const CLICKHOUSE_DB: &str = "tdw";
const QDRANT_COLLECTION: &str = "tdw-default";
const MEILI_INDEX: &str = "tdw-default";
const DEFAULT_QDRANT_VECTOR_SIZE: usize = 1536;

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

    // Optional search/OLAP/vector schema bootstrap. Each backend is skipped
    // unless its endpoint env var is set, so the minimal Postgres + S3 live
    // profile keeps working unchanged.
    if let Ok(clickhouse_url) = env::var("TDW_CLICKHOUSE_URL") {
        let user = env::var("TDW_CLICKHOUSE_USER").ok();
        let password = env::var("TDW_CLICKHOUSE_PASSWORD").ok();
        let engine = match ClickHouseHttpEngine::new(&clickhouse_url, user, password) {
            Ok(engine) => engine,
            Err(error) => {
                log_step("clickhouse-connect", "failed", Some(&error.to_string()));
                return ExitCode::from(7);
            }
        };
        let statements = [
            format!("create database if not exists {CLICKHOUSE_DB}"),
            format!(
                "create table if not exists {CLICKHOUSE_DB}._tdw_bootstrap_marker \
                 (key String, created_at DateTime default now()) \
                 engine = MergeTree order by key"
            ),
        ];
        for statement in &statements {
            if let Err(error) = engine.execute(statement).await {
                log_step("clickhouse-schema", "failed", Some(&error.to_string()));
                return ExitCode::from(7);
            }
        }
        log_step(
            "clickhouse-schema",
            "ok",
            Some("database tdw + _tdw_bootstrap_marker"),
        );
    }

    if let Ok(qdrant_url) = env::var("TDW_QDRANT_URL") {
        let api_key = env::var("TDW_QDRANT_API_KEY").ok();
        let vector_size = env::var("TDW_QDRANT_VECTOR_SIZE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(DEFAULT_QDRANT_VECTOR_SIZE);
        let engine = match QdrantHttpEngine::new(&qdrant_url, api_key) {
            Ok(engine) => engine,
            Err(error) => {
                log_step("qdrant-connect", "failed", Some(&error.to_string()));
                return ExitCode::from(8);
            }
        };
        if let Err(error) = engine
            .ensure_collection(QDRANT_COLLECTION, vector_size)
            .await
        {
            log_step("qdrant-collection", "failed", Some(&error.to_string()));
            return ExitCode::from(8);
        }
        log_step(
            "qdrant-collection",
            "ok",
            Some(&format!("{QDRANT_COLLECTION} (size {vector_size})")),
        );
    }

    if let Ok(meili_url) = env::var("TDW_MEILI_URL") {
        let api_key = env::var("TDW_MEILI_API_KEY").ok();
        let engine = match MeilisearchHttpEngine::new(&meili_url, api_key) {
            Ok(engine) => engine,
            Err(error) => {
                log_step("meili-connect", "failed", Some(&error.to_string()));
                return ExitCode::from(9);
            }
        };
        if let Err(error) = engine.ensure_index(MEILI_INDEX, "id").await {
            log_step("meili-index", "failed", Some(&error.to_string()));
            return ExitCode::from(9);
        }
        log_step("meili-index", "ok", Some(MEILI_INDEX));
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
