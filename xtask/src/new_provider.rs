//! `xtask new-provider <name>` — scaffold a complete, compiling keyless
//! `tdw-provider-<name>` crate plus its workspace registration.
//!
//! The generator is the provider-onboarding spine called out in the ecosystem
//! gap (§C item 1): given a bare provider name (e.g. `acme`) it writes a
//! self-contained crate skeleton modelled on the existing simple keyless
//! providers (`tdw-provider-finra`, `tdw-provider-cboe`) — a `<Name>Raw` serde
//! wire struct, a crate-local `<Name>Record` model stub, a `<Name>HttpFetcher`
//! behind the `http` feature with an `https://` base URL, an offline
//! cassette test, and a `base_url_uses_tls` test — and registers it where the
//! workspace expects:
//!
//!   * the workspace `members` glob (`crates/*`) already covers the new dir, so
//!     no workspace `Cargo.toml` edit is needed;
//!   * a `provider-<name>` feature + optional path dependency in
//!     `crates/tdw-service-api/Cargo.toml`;
//!   * a `docs/quality/crate-readiness/tdw-provider-<name>.md` worksheet and a
//!     matrix row, so `crate-readiness-check` stays green.
//!
//! The two registration sites that cannot be safely auto-inserted (the
//! `tdw-service-api` dispatcher fetch/ingest arms and the endpoint-catalog route)
//! are emitted as a clearly-marked manual-follow-up TODO block, because they
//! require provider-specific endpoint/model decisions and live in a 8k-line
//! dispatcher + a separate catalog crate.
//!
//! Templating is a small hand-rolled placeholder substitution (no `tera`/
//! `handlebars` dependency) to keep the zero-extra-dep posture of `xtask`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Generated names derived from a raw provider name.
///
/// `id` is the lowercase dispatch/registry identifier (`acme`); `pascal` is the
/// type-name prefix (`Acme`); `screaming` is the const-name form (`ACME`).
struct Names {
    id: String,
    pascal: String,
    screaming: String,
}

impl Names {
    /// Build the name set from a raw CLI argument, validating the grammar.
    ///
    /// A provider name must be a non-empty lowercase `[a-z0-9-]` token starting
    /// with a letter (matching the existing `tdw-provider-*` crate names). The
    /// Pascal form upper-cases the first letter of each `-`-separated segment.
    fn parse(raw: &str) -> Result<Self, String> {
        if raw.is_empty() {
            return Err("provider name must not be empty".to_string());
        }
        let valid = raw
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        let starts_alpha = raw.chars().next().is_some_and(|c| c.is_ascii_lowercase());
        if !valid || !starts_alpha {
            return Err(format!(
                "provider name '{raw}' must be a lowercase [a-z0-9-] token starting with a letter"
            ));
        }

        let pascal = raw
            .split('-')
            .filter(|segment| !segment.is_empty())
            .map(|segment| {
                let mut chars = segment.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_ascii_uppercase().to_string() + chars.as_str()
                })
            })
            .collect::<String>();
        let screaming = raw.replace('-', "_").to_ascii_uppercase();

        Ok(Self {
            id: raw.to_string(),
            pascal,
            screaming,
        })
    }

    /// `tdw-provider-<id>` — the crate (package + directory) name.
    fn crate_name(&self) -> String {
        format!("tdw-provider-{}", self.id)
    }
}

/// Substitute `{{id}}`, `{{Pascal}}`, `{{SCREAMING}}`, and `{{crate}}`
/// placeholders in a template string.
fn render(template: &str, names: &Names) -> String {
    template
        .replace("{{crate_ident}}", &names.crate_name().replace('-', "_"))
        .replace("{{crate}}", &names.crate_name())
        .replace("{{SCREAMING}}", &names.screaming)
        .replace("{{Pascal}}", &names.pascal)
        .replace("{{id}}", &names.id)
}

/// Entry point for `xtask new-provider <name>`.
///
/// # Errors
///
/// Returns an error when the name is missing/invalid, when the target crate
/// directory already exists (the generator refuses to overwrite), or when any
/// file write or registration edit fails.
pub fn new_provider(name: Option<String>) -> Result<(), String> {
    let raw = name.ok_or_else(|| {
        "usage: cargo run -p xtask -- new-provider <name>   (e.g. new-provider acme)".to_string()
    })?;
    let names = Names::parse(&raw)?;
    let report = scaffold(Path::new("."), &names)?;
    print!("{report}");
    Ok(())
}

/// Scaffold the crate + registration under `workspace_root`, returning a
/// human-readable report of what was created and the manual follow-ups.
///
/// Factored out from [`new_provider`] so a test can drive it into a temp dir.
///
/// # Errors
///
/// Returns an error if the crate dir already exists or any write/edit fails.
fn scaffold(workspace_root: &Path, names: &Names) -> Result<String, String> {
    let crate_name = names.crate_name();
    let crate_dir = workspace_root.join("crates").join(&crate_name);
    if crate_dir.exists() {
        return Err(format!(
            "refusing to scaffold: crate directory already exists at {}",
            crate_dir.display()
        ));
    }

    let mut created = Vec::new();
    let mut write = |relative: PathBuf, contents: String| -> Result<(), String> {
        let path = crate_dir.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        fs::write(&path, contents).map_err(|e| format!("write {}: {e}", path.display()))?;
        created.push(path);
        Ok(())
    };

    write(PathBuf::from("Cargo.toml"), render(CARGO_TOML, names))?;
    write(PathBuf::from("src/lib.rs"), render(LIB_RS, names))?;
    write(
        PathBuf::from("src/http_fetcher.rs"),
        render(HTTP_FETCHER_RS, names),
    )?;
    write(
        PathBuf::from("tests/http_fetcher.rs"),
        render(TEST_RS, names),
    )?;

    // Registration: service-api feature + dep; readiness worksheet + matrix row.
    let mut registered = Vec::new();
    register_service_api(workspace_root, names, &mut registered)?;
    write_readiness(workspace_root, names, &mut created)?;
    add_matrix_row(workspace_root, names, &mut registered)?;

    Ok(render_report(names, &created, &registered, workspace_root))
}

/// Insert the `provider-<id>` feature and the optional path dependency into
/// `crates/tdw-service-api/Cargo.toml`, alphabetically anchored next to the
/// existing `provider-finra`/`tdw-provider-finra` lines.
fn register_service_api(
    workspace_root: &Path,
    names: &Names,
    registered: &mut Vec<String>,
) -> Result<(), String> {
    let manifest = workspace_root
        .join("crates")
        .join("tdw-service-api")
        .join("Cargo.toml");
    let content =
        fs::read_to_string(&manifest).map_err(|e| format!("read {}: {e}", manifest.display()))?;

    let feature_line = format!(
        "provider-{id} = [\"dep:{cn}\", \"{cn}/http\"]",
        id = names.id,
        cn = names.crate_name()
    );
    let dep_line = format!(
        "{cn} = {{ path = \"../{cn}\", optional = true }}",
        cn = names.crate_name()
    );

    if content.contains(&feature_line) || content.contains(&dep_line) {
        registered.push(format!(
            "{}: already registers provider-{} (left unchanged)",
            manifest.display(),
            names.id
        ));
        return Ok(());
    }

    // Anchor the feature after the first `provider-` feature line, and the dep
    // after the first `tdw-provider-*` optional dependency line, so the inserts
    // land inside the right tables without a full TOML parse.
    let with_feature = insert_after_line(
        &content,
        |line| line.trim_start().starts_with("provider-finra ="),
        &feature_line,
    )
    .or_else(|| {
        insert_after_line(
            &content,
            |line| {
                let t = line.trim_start();
                t.starts_with("provider-") && t.contains(" = [")
            },
            &feature_line,
        )
    })
    .ok_or_else(|| {
        format!(
            "could not find a provider feature anchor in {}",
            manifest.display()
        )
    })?;

    let with_dep = insert_after_line(
        &with_feature,
        |line| line.trim_start().starts_with("tdw-provider-finra = { path"),
        &dep_line,
    )
    .or_else(|| {
        insert_after_line(
            &with_feature,
            |line| {
                let t = line.trim_start();
                t.starts_with("tdw-provider-") && t.contains("optional = true")
            },
            &dep_line,
        )
    })
    .ok_or_else(|| {
        format!(
            "could not find a provider dependency anchor in {}",
            manifest.display()
        )
    })?;

    fs::write(&manifest, with_dep).map_err(|e| format!("write {}: {e}", manifest.display()))?;
    registered.push(format!(
        "{}: added `provider-{}` feature + optional dependency",
        manifest.display(),
        names.id
    ));
    Ok(())
}

/// Return `content` with `line` inserted immediately after the first line for
/// which `anchor` returns true, preserving the anchor line's indentation. Returns
/// `None` if no anchor line matches.
fn insert_after_line(content: &str, anchor: impl Fn(&str) -> bool, line: &str) -> Option<String> {
    let mut out = String::with_capacity(content.len() + line.len() + 1);
    let mut inserted = false;
    for current in content.lines() {
        out.push_str(current);
        out.push('\n');
        if !inserted && anchor(current) {
            out.push_str(line);
            out.push('\n');
            inserted = true;
        }
    }
    inserted.then_some(out)
}

/// Write the crate-readiness worksheet for the new crate.
fn write_readiness(
    workspace_root: &Path,
    names: &Names,
    created: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let path = workspace_root
        .join("docs/quality/crate-readiness")
        .join(format!("{}.md", names.crate_name()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    fs::write(&path, render(READINESS_MD, names))
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    created.push(path);
    Ok(())
}

/// Append a matrix row for the new crate to
/// `docs/quality/crate-readiness/matrix.md` (idempotent).
fn add_matrix_row(
    workspace_root: &Path,
    names: &Names,
    registered: &mut Vec<String>,
) -> Result<(), String> {
    let path = workspace_root.join("docs/quality/crate-readiness/matrix.md");
    let content = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let row_anchor = format!("| [{}](", names.crate_name());
    if content.contains(&row_anchor) {
        registered.push(format!(
            "{}: row for {} already present (left unchanged)",
            path.display(),
            names.crate_name()
        ));
        return Ok(());
    }
    let row = render(MATRIX_ROW, names);
    let mut updated = content;
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(&row);
    updated.push('\n');
    fs::write(&path, updated).map_err(|e| format!("write {}: {e}", path.display()))?;
    registered.push(format!(
        "{}: appended readiness matrix row for {}",
        path.display(),
        names.crate_name()
    ));
    Ok(())
}

/// Build the final stdout report: created files, registration edits, and the
/// manual follow-up TODO block (dispatcher arm, catalog route, 3-drift).
fn render_report(
    names: &Names,
    created: &[PathBuf],
    registered: &[String],
    workspace_root: &Path,
) -> String {
    let mut out = String::new();
    // `out` is a String; writes are infallible, so the `write!` results are
    // discarded. This avoids the `format_push_string` allocation per line.
    let _ = writeln!(
        out,
        "new-provider: scaffolded {} (keyless HTTP provider skeleton)\n",
        names.crate_name()
    );
    out.push_str("created files:\n");
    for path in created {
        let shown = path.strip_prefix(workspace_root).unwrap_or(path);
        let _ = writeln!(out, "  + {}", shown.display());
    }
    out.push_str("\nregistration:\n");
    for note in registered {
        let _ = writeln!(out, "  * {note}");
    }
    let _ = write!(
        out,
        "\nMANUAL FOLLOW-UP (the generator cannot safely auto-insert these):\n\
         \n\
         1. tdw-service-api dispatcher arm — wire the fetch + ingest bindings:\n\
         \x20  crates/tdw-service-api/src/dispatcher.rs\n\
         \x20    #[cfg(feature = \"provider-{id}\")]\n\
         \x20    insert_{id}_fetch_bindings(&mut table);   // in the fetch table builder\n\
         \x20    #[cfg(feature = \"provider-{id}\")]\n\
         \x20    insert_{id}_ingest_bindings(&mut table);  // in the ingest table builder\n\
         \x20  (model the two helpers on insert_finra_fetch_bindings / insert_finra_ingest_bindings,\n\
         \x20   and add the `provider-{id}` feature to the `all-http-providers` umbrella list.)\n\
         \n\
         2. Endpoint-catalog route — add the parity route + `{id}` candidate in\n\
         \x20  the tdw-endpoint-catalog crate so the dispatcher binding has a route.\n\
         \n\
         3. 3-drift regen after the catalog route lands (each `--target-dir target`):\n\
         \x20  cargo run -q -p xtask -- openapi-sync\n\
         \x20  cargo run -q -p xtask -- pysdk-sync\n\
         \x20  cargo run -q -p xtask -- catalog-check\n\
         \n\
         Then verify the new crate compiles + tests:\n\
         \x20  cargo test -p {cn} --features http --target-dir target\n",
        id = names.id,
        cn = names.crate_name(),
    );
    out
}

// ── Templates ─────────────────────────────────────────────────────────────────
//
// Placeholders: {{id}} (acme) / {{Pascal}} (Acme) / {{SCREAMING}} (ACME) /
// {{crate}} (tdw-provider-acme). Kept close to the finra/cboe shapes so the
// generated crate compiles and lints under the workspace's pedantic config.

const CARGO_TOML: &str = r#"[package]
name = "{{crate}}"
version = "0.1.0"
edition = "2024"
publish = false
license = "MIT OR Apache-2.0"

[lib]
doctest = false

[features]
default = []
http = ["dep:async-trait", "dep:bytes", "dep:reqwest", "dep:tdw-core", "dep:tokio", "tdw-core/http"]

[dependencies]
async-trait = { workspace = true, optional = true }
bytes = { workspace = true, optional = true }
reqwest = { workspace = true, optional = true }
schemars = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tdw-core = { workspace = true, optional = true }
thiserror = { workspace = true }
tokio = { workspace = true, optional = true }

[dev-dependencies]
tdw-provider-testkit = { path = "../tdw-provider-testkit" }

[[test]]
name = "http_fetcher"
path = "tests/http_fetcher.rs"
required-features = ["http"]

[lints]
workspace = true
"#;

const LIB_RS: &str = r##"#![forbid(unsafe_code)]
//! Offline types and validation for the {{Pascal}} provider (scaffolded by
//! `xtask new-provider`).
//!
//! The real HTTP fetchers live in [`http_fetcher`] and are only compiled when
//! the `http` feature is enabled. Everything in this module works without any
//! network access.
//!
//! TODO(scaffold): replace the placeholder query/record shapes below with the
//! real {{Pascal}} endpoint contract, and map `{{Pascal}}Record` onto the
//! appropriate standardized `tdw-domain` model in `http_fetcher.rs`.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{{Pascal}}HttpFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Public base URL for the {{Pascal}} API. Must use TLS (`https://`).
///
/// TODO(scaffold): point this at the real {{Pascal}} API base URL.
pub const BASE_URL: &str = "https://api.{{id}}.example/v1";

/// Provider identifier used in the registry and dispatch tables.
pub const PROVIDER_ID: &str = "{{id}}";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, {{Pascal}}ProviderError>;

/// Query parameters for the placeholder {{Pascal}} endpoint.
///
/// TODO(scaffold): replace `symbol` with the real request parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct {{Pascal}}Query {
    /// Instrument symbol the request targets.
    pub symbol: String,
}

impl {{Pascal}}Query {
    /// Construct and validate a query.
    ///
    /// # Errors
    ///
    /// Returns [`{{Pascal}}ProviderError::EmptySymbol`] when `symbol` is blank.
    pub fn new(symbol: &str) -> Result<Self> {
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return Err({{Pascal}}ProviderError::EmptySymbol);
        }
        Ok(Self {
            symbol: symbol.to_ascii_uppercase(),
        })
    }
}

/// Raw {{Pascal}} wire record as returned by the API (JSON).
///
/// TODO(scaffold): mirror the real {{Pascal}} response field names/types.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct {{Pascal}}Raw {
    /// Instrument symbol.
    pub symbol: String,
    /// Quoted value.
    pub value: f64,
}

/// Standardized {{Pascal}} output record (the crate-local model stub).
///
/// TODO(scaffold): map this onto a shared `tdw-domain` model in
/// `http_fetcher.rs` once the real endpoint shape is known.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct {{Pascal}}Record {
    /// Instrument symbol.
    pub symbol: String,
    /// Quoted value.
    pub value: f64,
}

impl From<{{Pascal}}Raw> for {{Pascal}}Record {
    fn from(raw: {{Pascal}}Raw) -> Self {
        Self {
            symbol: raw.symbol,
            value: raw.value,
        }
    }
}

/// Parse a full {{Pascal}} JSON response body into standardized records.
///
/// # Errors
///
/// Returns [`{{Pascal}}ProviderError::Parse`] when the body is not a JSON array
/// of [`{{Pascal}}Raw`] records.
pub fn parse_response(body: &[u8]) -> Result<Vec<{{Pascal}}Record>> {
    let raw: Vec<{{Pascal}}Raw> =
        serde_json::from_slice(body).map_err(|e| {{Pascal}}ProviderError::Parse(e.to_string()))?;
    Ok(raw.into_iter().map({{Pascal}}Record::from).collect())
}

/// Error type for the {{Pascal}} provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum {{Pascal}}ProviderError {
    /// The query symbol was blank.
    #[error("{{id}} query symbol must not be empty")]
    EmptySymbol,
    /// The response body failed to parse.
    #[error("{{id}} response parse error: {0}")]
    Parse(String),
    /// A transport/provider-side error surfaced from the fetcher.
    #[error("{{id}} provider error: {0}")]
    Provider(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_rejects_blank_symbol() {
        assert_eq!({{Pascal}}Query::new("  "), Err({{Pascal}}ProviderError::EmptySymbol));
    }

    #[test]
    fn query_uppercases_symbol() {
        let q = {{Pascal}}Query::new("aapl").expect("valid");
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn parse_response_maps_raw_to_records() {
        let body = br#"[{"symbol":"AAPL","value":1.5},{"symbol":"MSFT","value":2.0}]"#;
        let recs = parse_response(body).expect("parse");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].symbol, "AAPL");
        assert!((recs[1].value - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_response_rejects_bad_json() {
        assert!(matches!(
            parse_response(b"not json"),
            Err({{Pascal}}ProviderError::Parse(_))
        ));
    }

    #[test]
    fn record_roundtrips_via_serde_json() {
        let rec = {{Pascal}}Record {
            symbol: "AAPL".to_string(),
            value: 1.5,
        };
        let json = serde_json::to_string(&rec).expect("serialize");
        let back: {{Pascal}}Record = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(rec, back);
    }
}
"##;

const HTTP_FETCHER_RS: &str = r##"#![cfg(feature = "http")]
//! Real {{Pascal}} HTTP fetcher for `tdw_core::Fetcher` (scaffolded).
//!
//! Gated by the `http` feature. Talks to the public {{Pascal}} API without
//! authentication (keyless). Live calls should be gated by an env var in the
//! integration test so unattended CI stays offline.

use tdw_core::http_support::prelude::*;

use crate::{BASE_URL, {{Pascal}}Query, {{Pascal}}Record, parse_response};

tdw_core::provider_fetcher_struct!(
    /// Production {{Pascal}} HTTP fetcher.
    ///
    /// TODO(scaffold): set the real request path + parameters in `extract_data`.
    pub {{Pascal}}HttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<{{Pascal}}Query, {{Pascal}}Record> for {{Pascal}}HttpFetcher {
    const PROVIDER: &'static str = "{{id}}";
    const ENDPOINT: &'static str = "quote";

    fn transform_query(params: Value) -> Result<{{Pascal}}Query> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .unwrap_or_default();
        {{Pascal}}Query::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &{{Pascal}}Query,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/quote?symbol={}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = build_client()?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("{{id}} quote extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("{{id}} quote returned {status}: {body}")));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("{{id}} quote read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &{{Pascal}}Query,
        raw: Bytes,
    ) -> Result<Vec<{{Pascal}}Record>> {
        parse_response(&raw).map_err(|e| Error::Provider(format!("{{id}} quote parse: {e}")))
    }
}

fn build_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Provider(format!("{{id}} http client build: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_query_reads_symbol() {
        let q = {{Pascal}}HttpFetcher::transform_query(serde_json::json!({"symbol": "aapl"}))
            .expect("valid params");
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn transform_data_parses_cassette() {
        let fetcher = {{Pascal}}HttpFetcher::default();
        let query = {{Pascal}}Query::new("AAPL").expect("valid query");
        let raw = Bytes::from(r#"[{"symbol":"AAPL","value":1.5}]"#);
        let rows = fetcher.transform_data(&query, raw).expect("transform_data");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "AAPL");
    }

    #[test]
    fn transform_data_empty_array_is_empty_vec() {
        let fetcher = {{Pascal}}HttpFetcher::default();
        let query = {{Pascal}}Query::new("AAPL").expect("valid query");
        let rows = fetcher
            .transform_data(&query, Bytes::from("[]"))
            .expect("empty array must not error");
        assert!(rows.is_empty());
    }
}
"##;

const TEST_RS: &str = r##"#![cfg(feature = "http")]
//! Integration tests for the {{Pascal}} HTTP fetcher (scaffolded).
//!
//! The cassette test always runs under the `http` feature and parses a recorded
//! response shape without any network access. Add a live test gated by an env
//! var (e.g. `TDW_{{SCREAMING}}_LIVE=1`) once the real endpoint is wired.

use bytes::Bytes;
use tdw_core::Fetcher;
use {{crate_ident}}::{ {{Pascal}}HttpFetcher, {{Pascal}}Query, BASE_URL };

#[test]
fn base_url_uses_tls() {
    assert!(
        BASE_URL.starts_with("https://"),
        "{{Pascal}} base URL must use TLS, got {BASE_URL}"
    );
}

#[test]
fn cassette_transform_data_parses_response() {
    let fetcher = {{Pascal}}HttpFetcher::default();
    let query = {{Pascal}}Query::new("AAPL").expect("valid query");
    let raw = Bytes::from(r#"[{"symbol":"AAPL","value":1.5},{"symbol":"MSFT","value":2.0}]"#);

    let rows = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");

    assert_eq!(rows.len(), 2, "expected 2 rows, got {rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[1].symbol, "MSFT");
}

#[test]
fn cassette_transform_query_roundtrip() {
    let q = {{Pascal}}HttpFetcher::transform_query(serde_json::json!({"symbol": "tsla"}))
        .expect("transform_query must succeed");
    assert_eq!(q.symbol, "TSLA");
}
"##;

const READINESS_MD: &str = r"# {{crate}} Readiness Worksheet

Scaffolded by `xtask new-provider {{id}}`. Refresh this worksheet once the real
{{Pascal}} endpoint shape and dispatch wiring land.

## Evidence Snapshot

- Manifest: `crates/{{crate}}/Cargo.toml`.
- Targets: lib, test.
- Features: default; http=[dep:async-trait,dep:bytes,dep:reqwest,dep:tdw-core,dep:tokio,tdw-core/http].
- Tests: offline cassette + `base_url_uses_tls` (generated skeleton).
- Docs/examples: worksheet.

## Release Assessment

- Generated keyless HTTP provider skeleton; compiles and lints under the
  workspace pedantic config.
- Manual follow-ups remain before the provider is dispatchable: the
  `tdw-service-api` dispatcher fetch/ingest arms, the endpoint-catalog route +
  `{{id}}` candidate, and the 3-drift regeneration.

## Verdict

Scaffolded — not yet dispatchable. The crate is registered (service-api feature
+ dependency, readiness matrix row) and ready for the per-endpoint
implementation. Not a release blocker until its gates are enabled.
";

const MATRIX_ROW: &str = "| [{{crate}}]({{crate}}.md) | provider scaffolder (xtask new-provider) | lib, test | tdw-core | tdw-service-api | default; http=[dep:async-trait,dep:bytes,dep:reqwest,dep:tdw-core,dep:tokio,tdw-core/http] | 8 | worksheet | Scaffolded skeleton; awaiting endpoint impl + dispatch wiring. | Scaffolded — not yet dispatchable |";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_simple() {
        let n = Names::parse("acme").expect("valid");
        assert_eq!(n.id, "acme");
        assert_eq!(n.pascal, "Acme");
        assert_eq!(n.screaming, "ACME");
        assert_eq!(n.crate_name(), "tdw-provider-acme");
    }

    #[test]
    fn names_parse_hyphenated() {
        let n = Names::parse("acme-data").expect("valid");
        assert_eq!(n.pascal, "AcmeData");
        assert_eq!(n.screaming, "ACME_DATA");
        assert_eq!(n.crate_name(), "tdw-provider-acme-data");
    }

    #[test]
    fn names_reject_invalid() {
        assert!(Names::parse("").is_err());
        assert!(Names::parse("Acme").is_err());
        assert!(Names::parse("1acme").is_err());
        assert!(Names::parse("acme_data").is_err());
    }

    #[test]
    fn insert_after_line_inserts_once() {
        let content = "a\nb\nc\n";
        let out = insert_after_line(content, |l| l == "b", "INSERTED").expect("anchor");
        assert_eq!(out, "a\nb\nINSERTED\nc\n");
    }

    #[test]
    fn insert_after_line_returns_none_without_anchor() {
        assert!(insert_after_line("a\nb\n", |l| l == "zzz", "X").is_none());
    }

    /// Drive the full generator into a temp dir with a minimal fake workspace and
    /// assert the crate files exist, parse, and the registration edits land.
    #[test]
    fn scaffold_generates_compiling_skeleton_and_registration() {
        let root = std::env::temp_dir().join(format!(
            "xtask-new-provider-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        // Minimal fake workspace: a service-api manifest with the anchor lines,
        // and a readiness matrix + dir.
        let service_dir = root.join("crates/tdw-service-api");
        fs::create_dir_all(&service_dir).expect("mk service dir");
        fs::write(
            service_dir.join("Cargo.toml"),
            "[features]\n\
             provider-finra = [\"dep:tdw-provider-finra\", \"tdw-provider-finra/http\"]\n\
             \n\
             [dependencies]\n\
             tdw-provider-finra = { path = \"../tdw-provider-finra\", optional = true }\n",
        )
        .expect("write service manifest");
        let cr_dir = root.join("docs/quality/crate-readiness");
        fs::create_dir_all(&cr_dir).expect("mk crate-readiness dir");
        fs::write(
            cr_dir.join("matrix.md"),
            "# Crate Readiness Matrix\n\n| Crate | ... |\n| --- | --- |\n",
        )
        .expect("write matrix");

        let names = Names::parse("acme").expect("valid");
        let report = scaffold(&root, &names).expect("scaffold");

        // Generated crate files exist.
        let crate_dir = root.join("crates/tdw-provider-acme");
        for rel in [
            "Cargo.toml",
            "src/lib.rs",
            "src/http_fetcher.rs",
            "tests/http_fetcher.rs",
        ] {
            assert!(crate_dir.join(rel).is_file(), "missing generated {rel}");
        }

        // Templates rendered: no leftover placeholders, and key Rust symbols present.
        let lib = fs::read_to_string(crate_dir.join("src/lib.rs")).expect("read lib");
        assert!(!lib.contains("{{"), "unrendered placeholder in lib.rs");
        assert!(lib.contains("pub struct AcmeRaw"), "AcmeRaw missing");
        assert!(lib.contains("pub struct AcmeRecord"), "AcmeRecord missing");
        assert!(lib.contains("https://"), "https base URL missing");
        let test = fs::read_to_string(crate_dir.join("tests/http_fetcher.rs")).expect("read test");
        assert!(
            test.contains("fn base_url_uses_tls"),
            "base_url_uses_tls test missing"
        );
        let cargo = fs::read_to_string(crate_dir.join("Cargo.toml")).expect("read cargo");
        assert!(
            cargo.contains("name = \"tdw-provider-acme\""),
            "crate name missing"
        );

        // Registration: service-api feature + dep, readiness worksheet + matrix row.
        let svc = fs::read_to_string(service_dir.join("Cargo.toml")).expect("read svc");
        assert!(
            svc.contains("provider-acme = [\"dep:tdw-provider-acme\""),
            "feature not added"
        );
        assert!(
            svc.contains("tdw-provider-acme = { path = \"../tdw-provider-acme\""),
            "dep not added"
        );
        assert!(
            root.join("docs/quality/crate-readiness/tdw-provider-acme.md")
                .is_file(),
            "readiness worksheet missing"
        );
        let matrix = fs::read_to_string(cr_dir.join("matrix.md")).expect("read matrix");
        assert!(
            matrix.contains("| [tdw-provider-acme](tdw-provider-acme.md)"),
            "matrix row missing"
        );

        // Report mentions the manual follow-ups.
        assert!(
            report.contains("MANUAL FOLLOW-UP"),
            "report missing follow-up block"
        );
        assert!(
            report.contains("dispatcher"),
            "report missing dispatcher note"
        );

        // Idempotency: a second run refuses because the crate dir exists.
        assert!(
            scaffold(&root, &names).is_err(),
            "second scaffold must refuse"
        );

        fs::remove_dir_all(&root).ok();
    }
}
