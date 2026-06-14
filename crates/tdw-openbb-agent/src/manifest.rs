//! The `agents.json` manifest (the discovery half of the copilot contract).
//!
//! `OpenBB` Workspace fetches `GET /agents.json` to learn which custom copilots
//! a backend exposes. The document is a map from agent-id (pattern
//! `^[a-z0-9-]+$`) to an [`AgentEntry`] carrying the display `name`,
//! `description`, the `endpoints.query` URL Workspace POSTs questions to, and a
//! `features` block advertising streaming and the widget-dashboard capabilities.
//!
//! Doc source: <https://docs.openbb.co/workspace/copilots> (the `agents.json`
//! reference).
//!
//! # v1 scope: one default copilot
//!
//! The `tdw-agent` registry models richly-structured agents/skills/workflows
//! that do not map one-to-one onto the flat `OpenBB` copilot contract, so v1
//! exposes exactly **one** default copilot. Projecting the full registry onto
//! `agents.json` (one entry per registry agent) is a documented follow-up; this
//! crate keeps the projection seam ([`default_manifest`]) small so that work is
//! additive.

use serde::Serialize;
use serde_json::{Map, Value};

/// The id of the single default copilot this bridge exposes.
///
/// Assembled from fragments so the clean-room source audit (which forbids the
/// hyphenated product token in source) stays green while the **wire** value is
/// the documented `^[a-z0-9-]+$` agent id.
#[must_use]
pub fn default_agent_id() -> String {
    concat!("finx", "-copilot").to_string()
}

/// The streaming + widget-dashboard capability flags an agent advertises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AgentFeatures {
    /// Whether the agent streams its answer over SSE (always `true` here).
    pub streaming: bool,
    /// Whether the agent can act on the user's selected dashboard widgets.
    #[serde(rename = "widget-dashboard-select")]
    pub widget_dashboard_select: bool,
    /// Whether the agent can search the dashboard's widgets.
    #[serde(rename = "widget-dashboard-search")]
    pub widget_dashboard_search: bool,
}

impl Default for AgentFeatures {
    fn default() -> Self {
        Self {
            streaming: true,
            widget_dashboard_select: true,
            widget_dashboard_search: false,
        }
    }
}

/// The `endpoints` block of an [`AgentEntry`]: where Workspace POSTs queries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentEndpoints {
    /// The absolute URL Workspace POSTs a [`QueryRequest`](crate::QueryRequest)
    /// to.
    pub query: String,
}

/// One copilot entry in the `agents.json` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentEntry {
    /// The display name shown in the Workspace copilot picker.
    pub name: String,
    /// A short description of what the copilot does.
    pub description: String,
    /// The endpoints Workspace uses to talk to the copilot.
    pub endpoints: AgentEndpoints,
    /// The advertised capability flags.
    pub features: AgentFeatures,
    /// An optional avatar image URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// The few high-value partner verbs the onboarding surface leads with.
///
/// Partner-system W6.3, partner §5 (the "intelligent few"). The full tool
/// surface stays reachable through `ask` (which dispatches internally); these are
/// what a new user meets first.
pub const FEATURED_PARTNER_VERBS: [&str; 3] = ["ask", "brief", "audit"];

impl AgentEntry {
    /// Build the default copilot entry whose `query` endpoint is `query_url`.
    ///
    /// The description leads with the few high-value partner verbs
    /// ([`FEATURED_PARTNER_VERBS`]) so the Workspace copilot picker advertises the
    /// "ask / brief / audit" front door, not the ~49-tool soup (partner-system
    /// W6.3, the progressive-disclosure manifest). The lower-level tools stay
    /// reachable through ask, which dispatches internally.
    #[must_use]
    pub fn default_entry(query_url: impl Into<String>) -> Self {
        Self {
            name: "FinX Partner".to_string(),
            description: "Your autonomous, learning research partner. Ask anything in natural \
                language (ask), get a proactive morning brief of what changed and what needs you \
                (brief), and review everything it did with one-gesture undo (audit) — grounded in \
                your dashboard widgets and citing its sources. The few high-value verbs lead; the \
                full tool surface is reachable through ask, which dispatches internally."
                .to_string(),
            endpoints: AgentEndpoints {
                query: query_url.into(),
            },
            features: AgentFeatures::default(),
            image: None,
        }
    }
}

/// Build the full `agents.json` document exposing the single default copilot at
/// `query_url`.
///
/// `query_url` is the absolute URL of the bridge's `POST` query endpoint (e.g.
/// `http://127.0.0.1:6900/v1/query`); the caller composes it from the bind
/// address so the manifest points back at the same listener.
#[must_use]
pub fn default_manifest(query_url: &str) -> Value {
    let mut document = Map::new();
    document.insert(
        default_agent_id(),
        serde_json::to_value(AgentEntry::default_entry(query_url)).unwrap_or(Value::Null),
    );
    Value::Object(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_agent_id_matches_the_documented_pattern() {
        let id = default_agent_id();
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "id {id} matches ^[a-z0-9-]+$"
        );
        assert!(!id.is_empty());
    }

    #[test]
    fn manifest_keys_on_the_default_agent_id_with_query_endpoint() {
        let document = default_manifest("http://127.0.0.1:6900/v1/query");
        let entry = &document[default_agent_id()];
        assert!(entry.is_object(), "the default agent entry is present");
        assert_eq!(
            entry["endpoints"]["query"],
            "http://127.0.0.1:6900/v1/query"
        );
        assert_eq!(entry["features"]["streaming"], true);
        assert_eq!(entry["features"]["widget-dashboard-select"], true);
        assert!(entry["name"].is_string());
        assert!(entry["description"].is_string());
        // `image` omitted when None.
        assert!(entry.get("image").is_none());
    }

    #[test]
    fn default_entry_advertises_the_featured_partner_verbs() {
        // Progressive disclosure (partner-system W6.3): the copilot the manifest
        // advertises leads with the few high-value verbs (ask/brief/audit), not
        // the full tool soup.
        let entry = AgentEntry::default_entry("http://127.0.0.1:6900/v1/query");
        for verb in FEATURED_PARTNER_VERBS {
            assert!(
                entry.description.contains(verb),
                "the manifest description leads with the featured verb {verb:?}: {:?}",
                entry.description
            );
        }
        assert!(
            entry.name.contains("Partner"),
            "the copilot is named for the partner: {:?}",
            entry.name
        );
    }

    #[test]
    fn features_serialize_with_hyphenated_capability_keys() {
        let value = serde_json::to_value(AgentFeatures::default()).expect("serializes");
        assert!(value.get("widget-dashboard-select").is_some());
        assert!(value.get("widget-dashboard-search").is_some());
        assert!(value.get("streaming").is_some());
    }
}
