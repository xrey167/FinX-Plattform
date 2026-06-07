#![forbid(unsafe_code)]

//! Email-marketing broadcast client — subscriber management and audience-wide
//! broadcasts with immediate-vs-draft semantics.
//!
//! # Feature gate
//!
//! The [`BroadcastClient`] and its HTTP transport are compiled only when the
//! `broadcast` feature is enabled. The data models ([`Subscriber`],
//! [`SubscriberList`], [`Broadcast`], [`BroadcastVisibility`],
//! [`BroadcastOutcome`]), [`BroadcastError`], and [`BroadcastConfig`] are
//! always compiled so callers can reference them without pulling in reqwest.
//!
//! # Configuration
//!
//! | Variable | Required | Purpose |
//! |----------|----------|---------|
//! | `TDW_BROADCAST_API_BASE` | yes | Base URL of the marketing API (e.g. `https://api.kit.com/v4`) |
//! | `TDW_BROADCAST_API_KEY` | yes | Bearer token; **never logged** |
//! | `TDW_BROADCAST_FORM_ID` | no | Default form/list ID used when none is supplied to `add_subscriber` |
//!
//! # Draft-detection heuristic
//!
//! Provider APIs (e.g. Kit/ConvertKit) return HTTP 201 with a JSON body
//! containing `"status": "draft"` when the sender address is unconfirmed.
//! A real send returns `"status": "published"` (or similar non-draft value).
//! The pure function [`classify_broadcast_response`] encodes this heuristic:
//! status 2xx + body contains `"status":"draft"` (whitespace-tolerant) →
//! [`BroadcastOutcome::SoftSuccessDraft`]; any other 2xx → [`BroadcastOutcome::Sent`];
//! non-2xx → [`BroadcastError::Api`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::BroadcastError;

// ---------------------------------------------------------------------------
// Models — always compiled, no I/O
// ---------------------------------------------------------------------------

/// A single marketing-list subscriber.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscriber {
    /// Subscriber's email address.
    pub email: String,
    /// Optional first name for personalisation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    /// Arbitrary string key→value custom fields (e.g. plan tier, signup source).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_fields: BTreeMap<String, String>,
}

/// A page of subscribers returned by the list endpoint.
///
/// `total` reflects the provider's full audience count even when the returned
/// page is smaller, enabling audit / reconciliation logic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriberList {
    /// Subscribers in this page.
    pub subscribers: Vec<Subscriber>,
    /// Total subscriber count as reported by the provider.
    pub total: u64,
}

/// Broadcast visibility scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadcastVisibility {
    /// Visible to the public (e.g. published to a web archive).
    Public,
    /// Visible only to the recipient list; not publicly archived.
    Private,
}

/// A broadcast email to send to the full subscriber audience.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Broadcast {
    /// Email subject line.
    pub subject: String,
    /// HTML body.
    pub html: String,
    /// Whether this broadcast is publicly archived or list-only.
    pub visibility: BroadcastVisibility,
    /// Optional Unix timestamp (milliseconds) for scheduled delivery.
    /// `None` means send immediately (or save as draft if unconfirmed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_ts_ms: Option<i64>,
}

/// Outcome of a [`BroadcastClient::send_broadcast`] call.
///
/// The `SoftSuccessDraft` variant is *not* an error — it means the provider
/// accepted the request but saved it as a draft because the sender address
/// has not yet been confirmed. Callers should log a warning and prompt the
/// user to verify their sender email rather than treating this as a failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BroadcastOutcome {
    /// The broadcast was dispatched immediately; `id` is the provider's broadcast ID.
    Sent {
        /// Provider-assigned broadcast identifier.
        id: String,
    },
    /// The provider accepted the payload but saved it as a draft because the
    /// sender is unconfirmed. `reason` carries the provider's status string.
    SoftSuccessDraft {
        /// Provider-assigned broadcast identifier.
        id: String,
        /// Provider status string explaining why it landed as a draft.
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// BroadcastConfig — always compiled
// ---------------------------------------------------------------------------

/// Connection parameters for the marketing broadcast API.
///
/// Construct via [`BroadcastConfig::from_env`]; returns `None` when the
/// mandatory variables are absent, indicating the transport is unconfigured
/// rather than misconfigured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BroadcastConfig {
    /// Base URL of the marketing API (no trailing slash).
    pub api_base: String,
    /// Bearer token. **Never log this value.**
    pub api_key: String,
    /// Default form/list ID used when the caller does not supply one.
    pub default_form_id: Option<String>,
}

impl BroadcastConfig {
    /// Read broadcast configuration from environment variables.
    ///
    /// Returns `None` when `TDW_BROADCAST_API_BASE` or `TDW_BROADCAST_API_KEY`
    /// is absent, meaning the transport is intentionally unconfigured for this
    /// deployment. Callers should disable the broadcast path rather than
    /// returning an error at startup.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let api_base = std::env::var("TDW_BROADCAST_API_BASE").ok()?;
        let api_key = std::env::var("TDW_BROADCAST_API_KEY").ok()?;
        let default_form_id = std::env::var("TDW_BROADCAST_FORM_ID").ok();

        Some(Self {
            api_base,
            api_key,
            default_form_id,
        })
    }
}

// ---------------------------------------------------------------------------
// classify_broadcast_response — pure, always compiled, fully testable
// ---------------------------------------------------------------------------

/// Classify a provider HTTP response into a [`BroadcastOutcome`] or a
/// [`BroadcastError`] without making any network calls.
///
/// # Draft-detection heuristic
///
/// Kit / ConvertKit-style APIs return **HTTP 2xx** with a JSON body that
/// contains `"status": "draft"` when the sender address has not been
/// confirmed. The function detects this by:
///
/// 1. Checking that `status` is in the 200–299 range.
/// 2. Searching the raw body for the pattern `"status"` followed (after
///    optional whitespace and `:`) by `"draft"`.  This text search avoids
///    a full JSON parse, making it testable and allocation-cheap.
/// 3. Extracting the provider broadcast `id` from a `"id"` JSON field (also
///    via text search / simple extraction).
///
/// Any 2xx response that is *not* draft-flagged is classified as
/// [`BroadcastOutcome::Sent`]. Non-2xx responses become [`BroadcastError::Api`].
///
/// # Errors
///
/// - [`BroadcastError::Api`] — non-2xx HTTP status.
/// - [`BroadcastError::Serialization`] — 2xx but the `id` field is missing or
///   unparseable (the provider returned an unexpected body shape).
pub fn classify_broadcast_response(
    status: u16,
    body: &str,
) -> Result<BroadcastOutcome, BroadcastError> {
    if !(200..300).contains(&status) {
        return Err(BroadcastError::Api {
            status,
            body: body.to_owned(),
        });
    }

    // Extract the `id` field from the JSON body using a lightweight text search.
    let id = extract_json_string_field(body, "id").ok_or_else(|| {
        BroadcastError::Serialization(format!(
            "broadcast response missing `id` field; body: {body}"
        ))
    })?;

    // Draft detection: look for `"status"` : `"draft"` in the body.
    if is_draft_status(body) {
        let reason =
            extract_json_string_field(body, "status").unwrap_or_else(|| "draft".to_owned());
        Ok(BroadcastOutcome::SoftSuccessDraft { id, reason })
    } else {
        Ok(BroadcastOutcome::Sent { id })
    }
}

/// Return `true` when the body contains a JSON `"status": "draft"` pattern
/// (whitespace-tolerant around the colon).
fn is_draft_status(body: &str) -> bool {
    // Walk the body looking for `"status"` followed by optional whitespace,
    // `:`, optional whitespace, then `"draft"`.
    let mut haystack = body;
    while let Some(pos) = haystack.find("\"status\"") {
        haystack = &haystack[pos + 8..]; // skip past `"status"`
        // Skip optional whitespace then `:`.
        let after = haystack.trim_start_matches(|c: char| c.is_whitespace() || c == ':');
        if after.trim_start().starts_with("\"draft\"") {
            return true;
        }
    }
    false
}

/// Extract the string value of a JSON key using a simple text scan.
///
/// Only handles `"key": "value"` patterns (no escapes inside the value).
/// Sufficient for the narrow use of extracting `id` and `status` from
/// well-behaved marketing API responses.
fn extract_json_string_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let pos = body.find(&needle)?;
    let after_key = &body[pos + needle.len()..];
    // Skip whitespace and colon.
    let after_colon = after_key
        .trim_start_matches(|c: char| c.is_whitespace() || c == ':')
        .trim_start();
    if !after_colon.starts_with('"') {
        return None; // value is not a JSON string
    }
    let inner = &after_colon[1..]; // skip opening quote
    let end = inner.find('"')?;
    Some(inner[..end].to_owned())
}

// ---------------------------------------------------------------------------
// BroadcastClient — compiled only with the `broadcast` feature
// ---------------------------------------------------------------------------

/// HTTP client for a Kit/Resend-style marketing broadcast API.
///
/// Construct via [`BroadcastClient::new`] after verifying that
/// [`BroadcastConfig::from_env`] returns `Some`. Reuse across the application
/// lifetime — the underlying reqwest client maintains a connection pool.
///
/// The Bearer token is stored in memory but **never written to logs or error
/// messages**.
#[cfg(feature = "broadcast")]
pub struct BroadcastClient {
    http: reqwest::Client,
    config: BroadcastConfig,
}

#[cfg(feature = "broadcast")]
impl BroadcastClient {
    /// Build a client from `config`.
    ///
    /// # Errors
    ///
    /// Returns [`BroadcastError::NotConfigured`] when `config` fields are
    /// empty (defensive; callers that use [`BroadcastConfig::from_env`] will
    /// not hit this in practice).
    pub fn new(config: BroadcastConfig) -> Result<Self, BroadcastError> {
        if config.api_base.is_empty() || config.api_key.is_empty() {
            return Err(BroadcastError::NotConfigured);
        }
        let http = reqwest::Client::new();
        Ok(Self { http, config })
    }

    /// Subscribe `sub` to the form/list identified by `form_id`.
    ///
    /// Uses the provider's subscribe-to-form endpoint:
    /// `POST {api_base}/forms/{form_id}/subscribe`.
    ///
    /// # Errors
    ///
    /// - [`BroadcastError::Http`] — network or TLS failure.
    /// - [`BroadcastError::Api`] — non-2xx response from the provider.
    /// - [`BroadcastError::Serialization`] — request body could not be serialised.
    pub async fn add_subscriber(
        &self,
        form_id: &str,
        sub: &Subscriber,
    ) -> Result<(), BroadcastError> {
        let url = format!("{}/forms/{}/subscribe", self.config.api_base, form_id);

        let body =
            serde_json::to_string(sub).map_err(|e| BroadcastError::Serialization(e.to_string()))?;

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| BroadcastError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            Ok(())
        } else {
            let resp_body = resp.text().await.unwrap_or_default();
            Err(BroadcastError::Api {
                status,
                body: resp_body,
            })
        }
    }

    /// Retrieve the current subscriber list.
    ///
    /// Uses the provider's subscribers endpoint: `GET {api_base}/subscribers`.
    ///
    /// # Errors
    ///
    /// - [`BroadcastError::Http`] — network or TLS failure.
    /// - [`BroadcastError::Api`] — non-2xx response from the provider.
    /// - [`BroadcastError::Serialization`] — response body could not be deserialised.
    pub async fn list_subscribers(&self) -> Result<SubscriberList, BroadcastError> {
        let url = format!("{}/subscribers", self.config.api_base);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|e| BroadcastError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        let resp_body = resp
            .text()
            .await
            .map_err(|e| BroadcastError::Http(e.to_string()))?;

        if !(200..300).contains(&status) {
            return Err(BroadcastError::Api {
                status,
                body: resp_body,
            });
        }

        // Parse the response. We expect a JSON object with at minimum:
        //   { "subscribers": [...], "total_subscribers": N }
        // (Kit v4 uses `total_subscribers`; fall back to array length.)
        let parsed: serde_json::Value = serde_json::from_str(&resp_body)
            .map_err(|e| BroadcastError::Serialization(e.to_string()))?;

        let subscribers: Vec<Subscriber> = parsed
            .get("subscribers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let total = parsed
            .get("total_subscribers")
            .or_else(|| parsed.get("total"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(subscribers.len() as u64);

        Ok(SubscriberList { subscribers, total })
    }

    /// Send a broadcast to the full subscriber audience.
    ///
    /// Uses the provider's broadcasts endpoint: `POST {api_base}/broadcasts`.
    ///
    /// Returns [`BroadcastOutcome::SoftSuccessDraft`] (not an error) when the
    /// provider accepts the request but saves it as a draft because the sender
    /// address is unconfirmed.
    ///
    /// # Errors
    ///
    /// - [`BroadcastError::Http`] — network or TLS failure.
    /// - [`BroadcastError::Api`] — non-2xx response (other than the draft case).
    /// - [`BroadcastError::Serialization`] — request or response body issue.
    pub async fn send_broadcast(&self, b: &Broadcast) -> Result<BroadcastOutcome, BroadcastError> {
        let url = format!("{}/broadcasts", self.config.api_base);

        let body =
            serde_json::to_string(b).map_err(|e| BroadcastError::Serialization(e.to_string()))?;

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| BroadcastError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        let resp_body = resp
            .text()
            .await
            .map_err(|e| BroadcastError::Http(e.to_string()))?;

        classify_broadcast_response(status, &resp_body)
    }
}

// ---------------------------------------------------------------------------
// Unit tests — no network; test pure classify fn, serde, and config
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- classify_broadcast_response ------------------------------------------

    #[test]
    fn classify_sent_on_200_with_id() {
        let body = r#"{"id":"bc_123","status":"published"}"#;
        let outcome = classify_broadcast_response(200, body).expect("should be Ok");
        assert_eq!(
            outcome,
            BroadcastOutcome::Sent {
                id: "bc_123".to_owned()
            }
        );
    }

    #[test]
    fn classify_sent_on_201_with_id() {
        let body = r#"{"id":"bc_456","status":"sent"}"#;
        let outcome = classify_broadcast_response(201, body).expect("should be Ok");
        assert_eq!(
            outcome,
            BroadcastOutcome::Sent {
                id: "bc_456".to_owned()
            }
        );
    }

    #[test]
    fn classify_soft_success_draft_on_201_draft_status() {
        let body = r#"{"id":"bc_789","status":"draft"}"#;
        let outcome = classify_broadcast_response(201, body).expect("should be Ok");
        assert_eq!(
            outcome,
            BroadcastOutcome::SoftSuccessDraft {
                id: "bc_789".to_owned(),
                reason: "draft".to_owned(),
            }
        );
    }

    #[test]
    fn classify_soft_success_draft_on_200_draft_status_with_whitespace() {
        // Provider may format JSON with spaces around the colon.
        let body = r#"{"id": "bc_w01", "status" : "draft"}"#;
        let outcome = classify_broadcast_response(200, body).expect("should be Ok");
        assert!(
            matches!(outcome, BroadcastOutcome::SoftSuccessDraft { ref id, .. } if id == "bc_w01"),
            "expected SoftSuccessDraft, got {outcome:?}"
        );
    }

    #[test]
    fn classify_api_error_on_401() {
        let body = r#"{"error":"unauthorized"}"#;
        let err = classify_broadcast_response(401, body).expect_err("should be Err");
        assert!(
            matches!(err, BroadcastError::Api { status: 401, .. }),
            "expected Api error, got {err:?}"
        );
    }

    #[test]
    fn classify_api_error_on_500() {
        let body = "internal server error";
        let err = classify_broadcast_response(500, body).expect_err("should be Err");
        assert!(
            matches!(err, BroadcastError::Api { status: 500, .. }),
            "expected Api 500 error, got {err:?}"
        );
    }

    #[test]
    fn classify_serialization_error_when_id_missing() {
        // 2xx but no `id` field in body.
        let body = r#"{"status":"published"}"#;
        let err = classify_broadcast_response(200, body).expect_err("should be Err");
        assert!(
            matches!(err, BroadcastError::Serialization(_)),
            "expected Serialization error, got {err:?}"
        );
    }

    #[test]
    fn classify_serialization_error_on_empty_body() {
        let err = classify_broadcast_response(200, "").expect_err("should be Err");
        assert!(
            matches!(err, BroadcastError::Serialization(_)),
            "expected Serialization error, got {err:?}"
        );
    }

    // -- Subscriber serde round-trip ------------------------------------------

    #[test]
    fn subscriber_serde_roundtrip_full() {
        let mut fields = BTreeMap::new();
        fields.insert("plan".to_owned(), "pro".to_owned());
        fields.insert("source".to_owned(), "landing".to_owned());
        let sub = Subscriber {
            email: "alice@example.com".to_owned(),
            first_name: Some("Alice".to_owned()),
            custom_fields: fields,
        };
        let json = serde_json::to_string(&sub).expect("serialize");
        let back: Subscriber = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(sub, back);
    }

    #[test]
    fn subscriber_serde_roundtrip_minimal() {
        let sub = Subscriber {
            email: "bob@example.com".to_owned(),
            first_name: None,
            custom_fields: BTreeMap::new(),
        };
        let json = serde_json::to_string(&sub).expect("serialize");
        let back: Subscriber = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(sub, back);
        // Optional fields should be absent from JSON when None/empty.
        assert!(!json.contains("first_name"), "first_name should be omitted");
        assert!(
            !json.contains("custom_fields"),
            "custom_fields should be omitted"
        );
    }

    #[test]
    fn subscriber_list_serde_roundtrip() {
        let list = SubscriberList {
            subscribers: vec![Subscriber {
                email: "c@example.com".to_owned(),
                first_name: None,
                custom_fields: BTreeMap::new(),
            }],
            total: 42,
        };
        let json = serde_json::to_string(&list).expect("serialize");
        let back: SubscriberList = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(list, back);
    }

    // -- Broadcast serde round-trip -------------------------------------------

    #[test]
    fn broadcast_serde_roundtrip_with_schedule() {
        let b = Broadcast {
            subject: "Weekly Digest".to_owned(),
            html: "<h1>Hello</h1>".to_owned(),
            visibility: BroadcastVisibility::Public,
            scheduled_ts_ms: Some(1_700_000_000_000),
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let back: Broadcast = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, back);
    }

    #[test]
    fn broadcast_serde_roundtrip_immediate() {
        let b = Broadcast {
            subject: "Flash Sale".to_owned(),
            html: "<p>50% off today</p>".to_owned(),
            visibility: BroadcastVisibility::Private,
            scheduled_ts_ms: None,
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let back: Broadcast = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, back);
        assert!(
            !json.contains("scheduled_ts_ms"),
            "scheduled_ts_ms should be omitted when None"
        );
    }

    #[test]
    fn broadcast_outcome_serde_roundtrip_sent() {
        let outcome = BroadcastOutcome::Sent {
            id: "x1".to_owned(),
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        let back: BroadcastOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(outcome, back);
    }

    #[test]
    fn broadcast_outcome_serde_roundtrip_draft() {
        let outcome = BroadcastOutcome::SoftSuccessDraft {
            id: "d1".to_owned(),
            reason: "draft".to_owned(),
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        let back: BroadcastOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(outcome, back);
    }

    // -- BroadcastVisibility serde --------------------------------------------

    #[test]
    fn broadcast_visibility_serializes_as_snake_case() {
        let pub_json = serde_json::to_string(&BroadcastVisibility::Public).expect("serialize");
        assert_eq!(pub_json, r#""public""#);
        let priv_json = serde_json::to_string(&BroadcastVisibility::Private).expect("serialize");
        assert_eq!(priv_json, r#""private""#);
    }

    // -- BroadcastConfig::from_env --------------------------------------------

    /// Test config env-reading without mutating the process environment by
    /// mirroring the from_env logic with explicit values (same pattern as
    /// the existing email_tests.rs EmailConfig tests).
    #[test]
    fn broadcast_config_from_env_cases() {
        // Missing both → None.
        assert!(
            config_from_values(None, None, None).is_none(),
            "missing base and key → None"
        );
        // Base present, key absent → None.
        assert!(
            config_from_values(Some("https://api.kit.com/v4"), None, None).is_none(),
            "missing key → None"
        );
        // Key present, base absent → None.
        assert!(
            config_from_values(None, Some("secret"), None).is_none(),
            "missing base → None"
        );
        // Both present, no form id.
        let cfg = config_from_values(Some("https://api.kit.com/v4"), Some("secret"), None)
            .expect("should be Some");
        assert_eq!(cfg.api_base, "https://api.kit.com/v4");
        assert_eq!(cfg.api_key, "secret");
        assert!(cfg.default_form_id.is_none());
        // All three set.
        let cfg = config_from_values(
            Some("https://api.example.com"),
            Some("tok_abc"),
            Some("form_99"),
        )
        .expect("should be Some with form id");
        assert_eq!(cfg.default_form_id, Some("form_99".to_owned()));
    }

    /// Mirror of [`BroadcastConfig::from_env`] operating on explicit values
    /// to avoid mutating the process environment during parallel test runs.
    fn config_from_values(
        base: Option<&str>,
        key: Option<&str>,
        form_id: Option<&str>,
    ) -> Option<BroadcastConfig> {
        let api_base = base?.to_owned();
        let api_key = key?.to_owned();
        let default_form_id = form_id.map(str::to_owned);
        Some(BroadcastConfig {
            api_base,
            api_key,
            default_form_id,
        })
    }

    // -- BroadcastError display -----------------------------------------------

    #[test]
    fn broadcast_error_not_configured_display() {
        let err = BroadcastError::NotConfigured;
        assert!(err.to_string().contains("not configured"));
    }

    #[test]
    fn broadcast_error_api_display_includes_status_and_body() {
        let err = BroadcastError::Api {
            status: 422,
            body: "Unprocessable".to_owned(),
        };
        let msg = err.to_string();
        assert!(msg.contains("422"), "should include status: {msg}");
        assert!(msg.contains("Unprocessable"), "should include body: {msg}");
    }

    #[test]
    fn broadcast_error_http_display() {
        let err = BroadcastError::Http("connection refused".to_owned());
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn broadcast_error_serialization_display() {
        let err = BroadcastError::Serialization("bad json".to_owned());
        assert!(err.to_string().contains("bad json"));
    }
}
