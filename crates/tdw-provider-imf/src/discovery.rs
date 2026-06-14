//! Clean-room catalog, query types, and SDMX discovery decoding for the
//! `imf_utils/*` helpers (OpenBB-parity **P4W9**).
//!
//! Every fact here is from the IMF Data Services SDMX-JSON documentation at
//! <https://datahelp.imf.org/knowledgebase/articles/667681-using-json-restful-web-service>:
//! the `Dataflow` discovery method (lists every available dataflow with its
//! `KeyFamilyRef` `DataStructure` id) and the `DataStructure/{id}` method (returns
//! the dimensions of a dataflow's key family). These shapes are distinct from the
//! `CompactData` observation envelope the macro fetcher consumes, so the helpers
//! carry their own query type, fetcher, and the long-format
//! [`tdw_domain::ImfDiscoveryRecord`] model. The API is keyless.
//!
//! This module is dependency-free (no `http` feature) so the catalog and the
//! pure decoders compile in the default offline workspace build.

/// One standardized `imf_utils/*` discovery helper.
///
/// `command` is the `imf_utils/*` command path it standardizes. `method` is the
/// SDMX-JSON REST method it calls (`Dataflow` or `DataStructure`); `kind` is the
/// [`tdw_domain::ImfDiscoveryRecord`] `kind` discriminator the decoded rows
/// carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImfUtilsEndpoint {
    /// `imf_utils/*` command path this endpoint standardizes.
    pub command: &'static str,
    /// SDMX-JSON REST method: `"Dataflow"` or `"DataStructure"`.
    pub method: &'static str,
    /// `ImfDiscoveryRecord::kind` discriminator the decoded rows carry.
    pub kind: &'static str,
    /// Whether the helper requires a caller-supplied `dataflow` id.
    pub requires_dataflow: bool,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// The standardized `imf_utils/*` discovery endpoint catalog.
///
/// `list_dataflows` and `list_tables` both read the `Dataflow` method (the IMF
/// service has no separate "tables" concept — a dataflow *is* the table grain —
/// so `list_tables` is `list_dataflows` filtered to the caller's `query`/`dataflow`
/// prefix). `get_dataflow_dimensions` and `presentation_table` both read the
/// `DataStructure/{dataflow}` method; the presentation helper additionally tags
/// each dimension cell as a presentation row.
pub const ENDPOINTS: &[ImfUtilsEndpoint] = &[
    ImfUtilsEndpoint {
        command: "imf_utils/list_dataflows",
        method: "Dataflow",
        kind: "dataflow",
        requires_dataflow: false,
        description: "List available IMF SDMX dataflows (SDMX-JSON Dataflow discovery, keyless).",
    },
    ImfUtilsEndpoint {
        command: "imf_utils/list_tables",
        method: "Dataflow",
        kind: "table",
        requires_dataflow: false,
        description: "List IMF SDMX dataflow tables, optionally filtered by id prefix (keyless).",
    },
    ImfUtilsEndpoint {
        command: "imf_utils/get_dataflow_dimensions",
        method: "DataStructure",
        kind: "dimension",
        requires_dataflow: true,
        description: "Get the SDMX key dimensions of an IMF dataflow (DataStructure, keyless).",
    },
    ImfUtilsEndpoint {
        command: "imf_utils/presentation_table",
        method: "DataStructure",
        kind: "presentation_cell",
        requires_dataflow: true,
        description: "Build an IMF dataflow presentation table from its SDMX dimensions (keyless).",
    },
];

/// Resolve a catalog entry by its `imf_utils/*` `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static ImfUtilsEndpoint> {
    ENDPOINTS.iter().find(|entry| entry.command == command)
}

/// One decoded SDMX discovery record, mirroring [`tdw_domain::ImfDiscoveryRecord`]
/// but kept dependency-free here. The fetcher maps these into the domain model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedRecord {
    /// `kind` discriminator (`"dataflow"`, `"table"`, `"dimension"`,
    /// `"presentation_cell"`).
    pub kind: String,
    /// Primary identifier.
    pub id: String,
    /// Human-readable name, when reported.
    pub name: Option<String>,
    /// Owning dataflow id, when applicable.
    pub dataflow: Option<String>,
    /// Backing `DataStructure` (DSD) id, when reported.
    pub structure: Option<String>,
    /// Dimension position in the SDMX key order (dimensions only).
    pub position: Option<u32>,
    /// Presentation-cell value (presentation rows only).
    pub value: Option<String>,
}

/// Read a possibly-prefixed SDMX text node. The SDMX-JSON service renders text as
/// either a bare string or a `{ "#text": "...", "@xml:lang": "en" }` object;
/// this collapses both, preferring the English text when an array is given.
fn sdmx_text(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(s)) => non_empty(s),
        Some(serde_json::Value::Object(map)) => map
            .get("#text")
            .and_then(serde_json::Value::as_str)
            .and_then(non_empty),
        Some(serde_json::Value::Array(items)) => {
            items.iter().find_map(|item| sdmx_text(Some(item)))
        }
        _ => None,
    }
}

/// Trim a string and treat the empty result as absent.
fn non_empty(s: &str) -> Option<String> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Read an SDMX attribute string (e.g. `@id`), treating empty as absent.
fn attr(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .and_then(non_empty)
}

/// Normalize a value SDMX renders as *either* a single object *or* an array of
/// objects into a borrowed slice. Mirrors the `CompactData` decoder's helper.
fn as_object_seq(value: Option<&serde_json::Value>) -> Vec<&serde_json::Value> {
    match value {
        Some(serde_json::Value::Array(items)) => items.iter().collect(),
        Some(item @ serde_json::Value::Object(_)) => vec![item],
        _ => Vec::new(),
    }
}

/// Decode the SDMX-JSON `Structure/Dataflows/Dataflow` list into discovery
/// records of the given `kind`.
///
/// Each dataflow carries its `@id`, its localized `Name`, and the
/// `KeyFamilyRef`/`Ref` `DataStructure` id. When `filter` is set (the
/// `list_tables` / `query` prefix) only dataflows whose id or name contains it
/// (case-insensitive) are kept.
#[must_use]
pub fn decode_dataflows(
    root: &serde_json::Value,
    kind: &str,
    filter: Option<&str>,
) -> Vec<DecodedRecord> {
    let dataflows = root
        .get("Structure")
        .and_then(|s| s.get("Dataflows"))
        .and_then(|d| d.get("Dataflow"));
    let needle = filter.map(str::to_ascii_lowercase);
    let mut rows = Vec::new();
    for flow in as_object_seq(dataflows) {
        let Some(id) = attr(flow.get("@id")) else {
            continue;
        };
        let name = sdmx_text(flow.get("Name"));
        let structure = flow
            .get("KeyFamilyRef")
            .and_then(|kf| kf.get("KeyFamilyID").or_else(|| kf.get("@id")))
            .and_then(|v| attr(Some(v)).or_else(|| sdmx_text(Some(v))))
            .or_else(|| flow.get("Ref").and_then(|r| attr(r.get("@id"))));
        if let Some(needle) = needle.as_deref() {
            let hay = format!(
                "{} {}",
                id.to_ascii_lowercase(),
                name.as_deref().unwrap_or_default().to_ascii_lowercase()
            );
            if !hay.contains(needle) {
                continue;
            }
        }
        rows.push(DecodedRecord {
            kind: kind.to_string(),
            id: id.clone(),
            name,
            dataflow: Some(id),
            structure,
            position: None,
            value: None,
        });
    }
    rows
}

/// Decode the SDMX-JSON `Structure/KeyFamilies/KeyFamily/Components/Dimension`
/// list for `dataflow` into discovery records of the given `kind`.
///
/// Each dimension carries its `@conceptRef` id, its codelist-derived name, and
/// its 1-based position in the SDMX key order. For presentation rows the
/// dimension id is also written into `value` so the long table reads as a flat
/// presentation grid.
#[must_use]
pub fn decode_dimensions(
    root: &serde_json::Value,
    dataflow: &str,
    kind: &str,
) -> Vec<DecodedRecord> {
    let key_families = root
        .get("Structure")
        .and_then(|s| s.get("KeyFamilies"))
        .and_then(|k| k.get("KeyFamily"));
    let mut rows = Vec::new();
    for family in as_object_seq(key_families) {
        let structure = attr(family.get("@id"));
        let dimensions = family.get("Components").and_then(|c| c.get("Dimension"));
        for (index, dim) in as_object_seq(dimensions).into_iter().enumerate() {
            let Some(id) = attr(dim.get("@conceptRef")).or_else(|| attr(dim.get("@id"))) else {
                continue;
            };
            let name = sdmx_text(dim.get("Name")).or_else(|| attr(dim.get("@codelist")));
            let position = u32::try_from(index + 1).ok();
            let value = matches!(kind, "presentation_cell").then(|| id.clone());
            rows.push(DecodedRecord {
                kind: kind.to_string(),
                id,
                name,
                dataflow: Some(dataflow.to_string()),
                structure: structure.clone(),
                position,
                value,
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn endpoints_are_unique_and_well_formed() {
        use std::collections::BTreeSet;
        let mut seen = BTreeSet::new();
        for endpoint in ENDPOINTS {
            assert!(
                seen.insert(endpoint.command),
                "duplicate command: {}",
                endpoint.command
            );
            assert!(endpoint.command.starts_with("imf_utils/"));
            assert!(matches!(endpoint.method, "Dataflow" | "DataStructure"));
        }
        assert_eq!(ENDPOINTS.len(), 4);
    }

    #[test]
    fn resolve_finds_known_and_misses_unknown() {
        assert_eq!(
            resolve("imf_utils/list_dataflows").map(|e| e.method),
            Some("Dataflow")
        );
        assert_eq!(
            resolve("imf_utils/get_dataflow_dimensions").map(|e| e.requires_dataflow),
            Some(true)
        );
        assert!(resolve("imf_utils/bogus").is_none());
    }

    #[test]
    fn decode_dataflows_reads_id_name_and_structure() {
        let root = json!({
            "Structure": { "Dataflows": { "Dataflow": [
                {
                    "@id": "IFS",
                    "Name": { "#text": "International Financial Statistics", "@xml:lang": "en" },
                    "KeyFamilyRef": { "KeyFamilyID": "ECOFIN_DSD" }
                },
                {
                    "@id": "BOP",
                    "Name": "Balance of Payments",
                    "KeyFamilyRef": { "KeyFamilyID": "BOP_DSD" }
                }
            ] } }
        });
        let rows = decode_dataflows(&root, "dataflow", None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "IFS");
        assert_eq!(
            rows[0].name.as_deref(),
            Some("International Financial Statistics")
        );
        assert_eq!(rows[0].structure.as_deref(), Some("ECOFIN_DSD"));
        assert_eq!(rows[0].dataflow.as_deref(), Some("IFS"));

        // The list_tables filter keeps only matching ids/names.
        let filtered = decode_dataflows(&root, "table", Some("balance"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "BOP");
        assert_eq!(filtered[0].kind, "table");
    }

    #[test]
    fn decode_dimensions_reads_position_and_tags_presentation() {
        let root = json!({
            "Structure": { "KeyFamilies": { "KeyFamily": {
                "@id": "ECOFIN_DSD",
                "Components": { "Dimension": [
                    { "@conceptRef": "FREQ", "Name": "Frequency" },
                    { "@conceptRef": "REF_AREA", "@codelist": "CL_AREA" }
                ] }
            } } }
        });
        let dims = decode_dimensions(&root, "IFS", "dimension");
        assert_eq!(dims.len(), 2);
        assert_eq!(dims[0].id, "FREQ");
        assert_eq!(dims[0].position, Some(1));
        assert_eq!(dims[0].structure.as_deref(), Some("ECOFIN_DSD"));
        assert_eq!(dims[1].id, "REF_AREA");
        assert_eq!(dims[1].position, Some(2));
        // Codelist falls back to the name when no localized Name is present.
        assert_eq!(dims[1].name.as_deref(), Some("CL_AREA"));

        let cells = decode_dimensions(&root, "IFS", "presentation_cell");
        assert_eq!(cells[0].value.as_deref(), Some("FREQ"));
    }
}
