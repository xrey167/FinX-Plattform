//! Offline, no-network example for `tdw-bootstrap`.
//!
//! The bootstrap binary itself requires live backends, so this example does NOT
//! connect to anything. It inspects the environment the binary would read and
//! reports which required variables are set vs missing, which optional backends
//! would be attempted, and the step/exit-code contract — a safe dry-run preview
//! of what `tdw-bootstrap` would do.
//!
//! Run with: `cargo run -p tdw-bootstrap --example tdw_bootstrap_basic`

const REQUIRED: &[&str] = &[
    "TDW_POSTGRES_URL",
    "TDW_S3_ENDPOINT",
    "TDW_S3_BUCKET",
    "TDW_S3_ACCESS_KEY",
    "TDW_S3_SECRET_KEY",
];

/// Optional backends keyed by the env var that enables them.
const OPTIONAL_BACKENDS: &[(&str, &str)] = &[
    ("TDW_CLICKHOUSE_URL", "clickhouse-schema"),
    ("TDW_QDRANT_URL", "qdrant-collection"),
    ("TDW_MEILI_URL", "meili-index"),
];

fn main() {
    println!("== tdw-bootstrap dry-run preview (no connections made) ==");

    // Required variables: report presence without printing secret values.
    let missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|name| {
            std::env::var(name)
                .map_or(true, |v| v.trim().is_empty())
        })
        .collect();
    for name in REQUIRED {
        let present = std::env::var(name)
            .is_ok_and(|v| !v.trim().is_empty());
        println!(
            "required {name}: {}",
            if present { "set" } else { "MISSING" }
        );
    }

    // Optional backends: each is skipped unless its *_URL is set.
    for (name, step) in OPTIONAL_BACKENDS {
        let enabled = std::env::var(name)
            .is_ok_and(|v| !v.trim().is_empty());
        println!(
            "optional {name}: {} (step {step} {})",
            if enabled { "set" } else { "unset" },
            if enabled { "would run" } else { "skipped" },
        );
    }

    // The step/exit-code contract the binary follows.
    println!(
        "{}",
        serde_json::json!({
            "exit_codes": {
                "0": "success",
                "2": "env",
                "3": "postgres-connect",
                "4": "postgres-schema",
                "5": "s3-marker",
                "6": "s3-roundtrip",
                "7": "clickhouse",
                "8": "qdrant",
                "9": "meilisearch"
            }
        })
    );

    if missing.is_empty() {
        println!("preview: all required vars present; tdw-bootstrap could proceed.");
    } else {
        println!("preview: tdw-bootstrap would exit 2 (env); missing: {missing:?}");
    }
}
