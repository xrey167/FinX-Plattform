#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for synchronous portions of tdw-core's trait contracts.
// The async surface (Fetcher::fetch, Streamer::subscribe, Engine writes) is
// intentionally exercised through tdw-service-api's higher-level samples and is
// not duplicated here because adding tokio as a dev-dep would expand the change
// footprint beyond what's safe under a no-shell verification regime.
//
// IMPORTANT: This file was authored offline. Verify with
// `cargo test --package tdw-core` before merging.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tdw_core::{
    Credentials, Error, ErrorCode, OBBject, ProviderKind, ProviderRegistry, RegistryEntry,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Row {
    symbol: String,
}

#[test]
fn obbject_with_metadata_persists_multiple_keys() {
    let object = OBBject::new(
        vec![Row {
            symbol: "AAPL".to_string(),
        }],
        "fileset",
        "equity_historical",
    )
    .with_metadata("source", json!("golden"))
    .with_metadata("region", json!("us"))
    .with_metadata("rows_estimate", json!(1));

    assert_eq!(object.metadata.get("source"), Some(&json!("golden")));
    assert_eq!(object.metadata.get("region"), Some(&json!("us")));
    assert_eq!(object.metadata.get("rows_estimate"), Some(&json!(1)));
}

#[test]
fn obbject_with_metadata_overwrites_existing_key() {
    let object = OBBject::new(Vec::<Row>::new(), "fileset", "equity_historical")
        .with_metadata("source", json!("first"))
        .with_metadata("source", json!("second"));

    assert_eq!(object.metadata.get("source"), Some(&json!("second")));
}

#[test]
fn obbject_new_defaults_to_empty_metadata() {
    let object = OBBject::new(Vec::<Row>::new(), "fileset", "endpoint");
    assert!(object.metadata.is_empty());
    assert!(object.rows.is_empty());
    assert_eq!(object.provider, "fileset");
    assert_eq!(object.endpoint, "endpoint");
}

#[test]
fn registry_resolve_returns_none_for_unknown_combination() {
    let registry = ProviderRegistry::default();
    assert!(
        registry
            .resolve("ghost", "endpoint", ProviderKind::Fetcher)
            .is_none()
    );
}

#[test]
fn registry_distinguishes_fetcher_and_streamer_at_same_endpoint() {
    let mut registry = ProviderRegistry::default();
    registry
        .register(RegistryEntry::fetcher("mock", "equity_historical"))
        .unwrap_or_else(|error| panic!("fetcher registers: {error}"));
    registry
        .register(RegistryEntry::streamer("mock", "equity_historical"))
        .unwrap_or_else(|error| panic!("streamer registers: {error}"));

    assert!(registry.contains("mock", "equity_historical", ProviderKind::Fetcher));
    assert!(registry.contains("mock", "equity_historical", ProviderKind::Streamer));
    assert_eq!(registry.entries().len(), 2);
}

#[test]
fn registry_entries_returns_slice_in_insertion_order() {
    let mut registry = ProviderRegistry::default();
    registry
        .register(RegistryEntry::fetcher("a", "ep"))
        .unwrap_or_else(|error| panic!("a registers: {error}"));
    registry
        .register(RegistryEntry::fetcher("b", "ep"))
        .unwrap_or_else(|error| panic!("b registers: {error}"));
    registry
        .register(RegistryEntry::fetcher("c", "ep"))
        .unwrap_or_else(|error| panic!("c registers: {error}"));

    let entries = registry.entries();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].provider, "a");
    assert_eq!(entries[1].provider, "b");
    assert_eq!(entries[2].provider, "c");
}

#[test]
fn registry_rejects_duplicate_same_kind_returns_registry_error() {
    let mut registry = ProviderRegistry::default();
    let entry = RegistryEntry::fetcher("mock", "equity_historical");
    registry
        .register(entry.clone())
        .unwrap_or_else(|error| panic!("first register: {error}"));

    let err = registry.register(entry).expect_err("duplicate must fail");
    let message = err.to_string();
    assert!(
        message.contains("duplicate") && message.contains("mock"),
        "error should mention duplicate + provider, got: {message}"
    );
}

#[test]
fn error_display_includes_payload_for_each_variant() {
    let cases: [(Error, &str); 4] = [
        (
            Error::InvalidQuery("missing field".to_string()),
            "missing field",
        ),
        (
            Error::Provider("transient outage".to_string()),
            "transient outage",
        ),
        (Error::Storage("disk full".to_string()), "disk full"),
        (
            Error::Registry("duplicate provider".to_string()),
            "duplicate provider",
        ),
    ];
    for (err, needle) in cases {
        let rendered = err.to_string();
        assert!(
            rendered.contains(needle),
            "Error::{err:?} display should contain '{needle}', got: {rendered}"
        );
    }
}

#[test]
fn credentials_default_starts_with_all_keys_unset() {
    let creds = Credentials::default();
    assert!(creds.polygon_api_key.is_none());
    assert!(creds.openai_api_key.is_none());
    assert!(creds.google_api_key.is_none());
    assert!(creds.anthropic_api_key.is_none());
}

#[test]
fn credentials_clone_preserves_set_keys() {
    let creds = Credentials {
        polygon_api_key: Some("poly".to_string()),
        openai_api_key: None,
        google_api_key: Some("goog".to_string()),
        anthropic_api_key: Some("anthropic".to_string()),
    };
    let clone = creds;
    assert_eq!(clone.polygon_api_key.as_deref(), Some("poly"));
    assert_eq!(clone.openai_api_key, None);
    assert_eq!(clone.google_api_key.as_deref(), Some("goog"));
    assert_eq!(clone.anthropic_api_key.as_deref(), Some("anthropic"));
}

#[test]
fn registry_entry_fetcher_constructor_records_kind() {
    let entry = RegistryEntry::fetcher("p", "ep");
    assert_eq!(entry.kind, ProviderKind::Fetcher);
    assert_eq!(entry.provider, "p");
    assert_eq!(entry.endpoint, "ep");
}

#[test]
fn registry_entry_streamer_constructor_records_kind() {
    let entry = RegistryEntry::streamer("p", "ep");
    assert_eq!(entry.kind, ProviderKind::Streamer);
}

#[test]
fn obbject_round_trips_an_empty_collection() {
    let original: OBBject<Row> = OBBject::new(Vec::new(), "fileset", "empty");
    let encoded: Value =
        serde_json::to_value(&original).unwrap_or_else(|error| panic!("serialize: {error}"));
    let decoded: OBBject<Row> =
        serde_json::from_value(encoded).unwrap_or_else(|error| panic!("deserialize: {error}"));

    assert_eq!(decoded.rows.len(), 0);
    assert_eq!(decoded.provider, "fileset");
    assert_eq!(decoded.endpoint, "empty");
}

// ── G008 CORE1: stable error codes + source chains ───────────────────────────

/// Each variant maps to exactly one stable `ErrorCode`; the code string is the
/// canonical value documented in [`ErrorCode::as_str`].
#[test]
fn error_code_is_stable_for_every_variant() {
    let cases: &[(Error, ErrorCode, &str)] = &[
        (
            Error::InvalidQuery("x".to_string()),
            ErrorCode::InvalidQuery,
            "IQ_001",
        ),
        (
            Error::Provider("x".to_string()),
            ErrorCode::Provider,
            "PV_001",
        ),
        (
            Error::Storage("x".to_string()),
            ErrorCode::Storage,
            "ST_001",
        ),
        (
            Error::Registry("x".to_string()),
            ErrorCode::Registry,
            "RG_001",
        ),
    ];
    for (err, expected_code, expected_str) in cases {
        let code = err.code();
        assert_eq!(code, *expected_code, "wrong code for {err:?}: got {code:?}");
        assert_eq!(
            code.as_str(),
            *expected_str,
            "wrong code string for {err:?}: got {}",
            code.as_str()
        );
        // `Display` of the code must equal `as_str`.
        assert_eq!(
            code.to_string(),
            *expected_str,
            "Display != as_str for {code:?}"
        );
    }
}

/// The existing Display output must be unchanged — this test mirrors the
/// `error_display_includes_payload_for_each_variant` assertion but is
/// explicit about the *full* rendered string so regressions are caught.
#[test]
fn error_display_is_unchanged_after_enrichment() {
    let cases: &[(Error, &str)] = &[
        (
            Error::InvalidQuery("missing field".to_string()),
            "invalid query: missing field",
        ),
        (
            Error::Provider("transient outage".to_string()),
            "provider error: transient outage",
        ),
        (
            Error::Storage("disk full".to_string()),
            "storage error: disk full",
        ),
        (
            Error::Registry("duplicate provider".to_string()),
            "registry error: duplicate provider",
        ),
    ];
    for (err, expected_display) in cases {
        let rendered = err.to_string();
        assert_eq!(
            rendered, *expected_display,
            "Display changed for variant {err:?}"
        );
    }
}

/// Variants without a structured source return `None` from
/// `std::error::Error::source`.
#[test]
fn error_string_variants_have_no_source() {
    use std::error::Error as StdError;

    let variants: &[Error] = &[
        Error::InvalidQuery("q".to_string()),
        Error::Provider("p".to_string()),
        Error::Storage("s".to_string()),
        Error::Registry("r".to_string()),
    ];
    for err in variants {
        assert!(
            err.source().is_none(),
            "string variant {err:?} should have no source"
        );
    }
}

/// `ErrorCode` is `Copy` + `PartialEq` + `Hash` — required for use in metrics
/// labels and `HashMap` keys without cloning.
#[test]
fn error_code_is_copy_and_eq() {
    let code = ErrorCode::Storage;
    let copy = code;
    assert_eq!(code, copy);

    // Hash contract: equal values must hash identically.
    use std::collections::HashMap;
    let mut map: HashMap<ErrorCode, u32> = HashMap::new();
    map.insert(ErrorCode::Storage, 1);
    assert_eq!(map[&ErrorCode::Storage], 1);
}
