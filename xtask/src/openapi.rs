//! `OpenAPI` 3.1 document generation from the endpoint catalog.
//!
//! The document is assembled **programmatically** from
//! [`tdw_endpoint_catalog::catalog`] so it can never drift from the catalog
//! spine: each `Fetch` route becomes a `GET` path item under `/api/v1/<route>`,
//! its query parameters are derived from the route's `params_schema` (top-level
//! JSON-Schema `properties`), and its `200` response references the route's
//! model schema, deduplicated into `components.schemas` by schema title.
//!
//! Determinism: the builder writes everything through `serde_json::Value`, whose
//! object maps are `BTreeMap`-backed (no `preserve_order` feature), so
//! `to_string_pretty` emits keys in sorted order. Running the generator twice
//! over the same catalog yields byte-identical output, which is what the
//! `openapi-check` drift gate relies on.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

/// Repo-relative path the generated `OpenAPI` document is written to and the
/// server embeds via `include_str!`, so the served doc and the checked-in doc
/// cannot diverge.
pub const OPENAPI_DOC_PATH: &str = "docs/schemas/openapi.json";

/// The `/api/v1` route-family prefix every catalog route is exposed under.
const API_PREFIX: &str = "/api/v1/";

/// Build the full `OpenAPI` 3.1 document as a deterministic
/// [`serde_json::Value`].
///
/// Only `Fetch` routes are emitted (they are the ones with a provider that can
/// serve an HTTP `GET`); `Compute` routes carry no provider and are skipped.
#[must_use]
pub fn build_openapi_document() -> Value {
    let mut paths = Map::new();
    // Deduplicate component schemas by title across all routes.
    let mut components: BTreeMap<String, Value> = BTreeMap::new();

    for entry in tdw_endpoint_catalog::catalog() {
        if entry.kind != tdw_endpoint_catalog::EndpointKind::Fetch {
            continue;
        }

        let params_schema = serde_json::to_value((entry.params_schema)()).unwrap_or(Value::Null);
        let model_schema = serde_json::to_value((entry.model)()).unwrap_or(Value::Null);

        // Place the model schema in components, keyed by its title (or a
        // route-derived fallback), and reference it from the 200 response.
        let schema_name =
            schema_title(&model_schema).unwrap_or_else(|| route_to_schema_name(entry.route));
        components
            .entry(schema_name.clone())
            .or_insert(model_schema);

        let parameters = query_parameters(&params_schema);
        let operation = json!({
            "summary": entry.doc,
            "operationId": route_to_operation_id(entry.route),
            "tags": [route_namespace(entry.route)],
            "parameters": parameters,
            "responses": {
                "200": {
                    "description": "Standardized result envelope.",
                    "content": {
                        "application/json": {
                            "schema": {
                                "$ref": format!("#/components/schemas/{schema_name}"),
                            }
                        }
                    }
                },
                "400": { "description": "Unknown route or invalid query parameters." },
                "404": { "description": "Path is not a catalog route." },
                "502": { "description": "Every candidate provider failed." }
            }
        });

        let path_key = format!("{API_PREFIX}{}", entry.route);
        paths.insert(path_key, json!({ "get": operation }));
    }

    let schemas: Map<String, Value> = components.into_iter().collect();

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "FinX-Plattform REST API",
            "description": "Catalog-derived REST surface over the trading-data-warehouse daemon. Routes are data, generated from the endpoint catalog.",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "servers": [
            { "url": "http://127.0.0.1:7879", "description": "Local daemon HTTP/SSE bind (default)." }
        ],
        "paths": Value::Object(paths),
        "components": { "schemas": Value::Object(schemas) }
    })
}

/// Serialize the `OpenAPI` document to a pretty, newline-terminated, deterministic
/// JSON string suitable for writing to disk and embedding via `include_str!`.
#[must_use]
pub fn render_openapi_document() -> String {
    let doc = build_openapi_document();
    serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
}

/// Derive the query parameters for a route from its params schema's top-level
/// `properties`. Each property becomes a `query` parameter referencing the
/// property's own schema; `required` is honored when the schema lists it.
fn query_parameters(params_schema: &Value) -> Value {
    let Some(properties) = params_schema.get("properties").and_then(Value::as_object) else {
        return Value::Array(Vec::new());
    };
    let required: Vec<&str> = params_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    // BTreeMap keeps the parameter list in stable (sorted) name order.
    let mut by_name: BTreeMap<String, Value> = BTreeMap::new();
    for (name, schema) in properties {
        let description = schema
            .get("description")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let mut param = Map::new();
        param.insert("name".to_string(), Value::String(name.clone()));
        param.insert("in".to_string(), Value::String("query".to_string()));
        param.insert(
            "required".to_string(),
            Value::Bool(required.contains(&name.as_str())),
        );
        if let Some(description) = description {
            param.insert("description".to_string(), Value::String(description));
        }
        param.insert("schema".to_string(), schema.clone());
        by_name.insert(name.clone(), Value::Object(param));
    }

    Value::Array(by_name.into_values().collect())
}

/// Extract a schema's `title` (the schemars-emitted type name), if present.
fn schema_title(schema: &Value) -> Option<String> {
    schema
        .get("title")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

/// Fallback component name for a model schema with no `title`: the route in
/// PascalCase-ish form (`equity/price/historical` → `EquityPriceHistorical`).
fn route_to_schema_name(route: &str) -> String {
    let mut out = String::new();
    for segment in route.split(['/', '_']) {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// Stable `operationId` for a route (`equity/price/historical` →
/// `equity_price_historical`).
fn route_to_operation_id(route: &str) -> String {
    route.replace('/', "_")
}

/// The top-level namespace segment used as the operation's tag.
fn route_namespace(route: &str) -> &str {
    route.split('/').next().unwrap_or(route)
}

/// Write the generated `OpenAPI` document to [`OPENAPI_DOC_PATH`].
///
/// # Errors
///
/// Returns a message if the parent directory cannot be created or the file
/// cannot be written.
pub fn openapi_sync() -> Result<(), String> {
    let content = render_openapi_document();
    let path = Path::new(OPENAPI_DOC_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, &content).map_err(|error| error.to_string())?;
    let doc = build_openapi_document();
    let paths = doc
        .get("paths")
        .and_then(Value::as_object)
        .map_or(0, Map::len);
    let schemas = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(Value::as_object)
        .map_or(0, Map::len);
    println!("openapi-sync wrote {paths} paths, {schemas} schemas to {OPENAPI_DOC_PATH}");
    Ok(())
}

/// Regenerate the `OpenAPI` document and fail if it differs from the checked-in
/// copy (the drift gate).
///
/// # Errors
///
/// Returns an error if the checked-in document is missing/unreadable or is stale
/// versus the freshly generated one.
pub fn openapi_check() -> Result<(), String> {
    let expected = render_openapi_document();
    let current = std::fs::read_to_string(OPENAPI_DOC_PATH).map_err(|error| {
        format!("openapi document missing or unreadable at {OPENAPI_DOC_PATH}: {error}")
    })?;
    if current == expected {
        println!("openapi-check passed: {OPENAPI_DOC_PATH} is up to date");
        Ok(())
    } else {
        Err(format!(
            "openapi document is stale; run `cargo run -p xtask -- openapi-sync` and commit {OPENAPI_DOC_PATH}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_is_openapi_31_with_paths_under_api_v1() {
        let doc = build_openapi_document();
        assert_eq!(doc["openapi"], "3.1.0");
        let paths = doc["paths"].as_object().expect("paths object");
        assert!(!paths.is_empty(), "at least one fetch route is exposed");
        for key in paths.keys() {
            assert!(
                key.starts_with(API_PREFIX),
                "every path lives under {API_PREFIX}, got {key}"
            );
        }
        // The seeded equity route is present and references a component schema.
        let equity = &doc["paths"]["/api/v1/equity/price/historical"]["get"];
        assert!(equity.is_object(), "equity price/historical GET present");
        let schema_ref =
            equity["responses"]["200"]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .expect("200 schema $ref");
        assert!(schema_ref.starts_with("#/components/schemas/"));
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(render_openapi_document(), render_openapi_document());
    }

    #[test]
    fn query_parameters_are_derived_from_params_schema_properties() {
        let doc = build_openapi_document();
        let params = doc["paths"]["/api/v1/equity/price/historical"]["get"]["parameters"]
            .as_array()
            .expect("parameters array");
        // `interval` is a top-level StandardParams field, so it surfaces as a
        // query parameter; provider-specific args (e.g. `symbol`) are not in the
        // catalog params schema and are intentionally not emitted here.
        assert!(
            params.iter().any(|p| p["name"] == "interval"),
            "standard query parameters are derived from the params schema"
        );
        for param in params {
            assert_eq!(param["in"], "query");
        }
    }

    #[test]
    fn route_helpers_normalize_consistently() {
        assert_eq!(
            route_to_operation_id("equity/price/historical"),
            "equity_price_historical"
        );
        assert_eq!(
            route_to_schema_name("equity/price/historical"),
            "EquityPriceHistorical"
        );
        assert_eq!(route_namespace("equity/price/historical"), "equity");
    }
}
