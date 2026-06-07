//! Price-alert domain model, store trait, and in-memory reference implementation.
//!
//! The `postgres` feature adds [`PgAlertStore`] backed by `sqlx::PgPool`.
//! Without that feature the crate compiles with zero I/O dependencies and is
//! safe to pull into any offline context.

#![forbid(unsafe_code)]

use std::cmp::Reverse;
use std::str::FromStr;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "postgres")]
pub use pg::PgAlertStore;

/// Default time-to-live for a newly created alert: 90 days in milliseconds.
pub const DEFAULT_TTL_MS: i64 = 90 * 24 * 60 * 60 * 1_000;

// ---------------------------------------------------------------------------
// AlertDirection
// ---------------------------------------------------------------------------

/// Whether an alert fires when the price moves above or below the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertDirection {
    /// Fires when the market price rises strictly above `target_price`.
    Above,
    /// Fires when the market price falls strictly below `target_price`.
    Below,
}

impl AlertDirection {
    /// Return the canonical database text representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Above => "Above",
            Self::Below => "Below",
        }
    }
}

impl FromStr for AlertDirection {
    type Err = AlertStoreError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "Above" => Ok(Self::Above),
            "Below" => Ok(Self::Below),
            other => Err(AlertStoreError::UnknownDirection(other.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// PriceAlert
// ---------------------------------------------------------------------------

/// A persisted price-alert record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PriceAlert {
    /// Stable, caller-supplied identifier (e.g. a ULID or UUID string).
    pub id: String,
    /// Identifier of the owner who created the alert.
    pub owner_id: String,
    /// Normalized ticker symbol (ASCII-uppercase, trimmed).
    pub symbol: String,
    /// Price level at which the alert triggers.
    pub target_price: f64,
    /// Whether the alert fires on an upward or downward price move.
    pub condition: AlertDirection,
    /// `true` while the alert is eligible to fire.
    pub active: bool,
    /// `true` once the alert has been fired; immutable after that point.
    pub triggered: bool,
    /// Unix-epoch millisecond timestamp after which the alert expires.
    pub expires_at_ms: i64,
    /// Unix-epoch millisecond timestamp when the alert was created.
    pub created_at_ms: i64,
    /// Unix-epoch millisecond timestamp of the last state change.
    pub updated_at_ms: i64,
}

impl PriceAlert {
    /// Construct a new [`PriceAlert`] from a [`NewAlert`] specification.
    ///
    /// - `id` is caller-supplied (use a ULID or UUID string).
    /// - `now_ms` is the current Unix-epoch millisecond timestamp.
    /// - The symbol is normalized via [`tdw_symbology::normalize_symbol`].
    /// - If `expires_at_ms` is `None` in `new`, it defaults to
    ///   `now_ms + DEFAULT_TTL_MS` (90 days).
    #[must_use]
    pub fn new(spec: NewAlert, id: String, now_ms: i64) -> Self {
        let expires_at_ms = spec
            .expires_at_ms
            .unwrap_or_else(|| now_ms.saturating_add(DEFAULT_TTL_MS));
        Self {
            id,
            owner_id: spec.owner_id,
            symbol: tdw_symbology::normalize_symbol(&spec.symbol),
            target_price: spec.target_price,
            condition: spec.condition,
            active: true,
            triggered: false,
            expires_at_ms,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }
}

// ---------------------------------------------------------------------------
// NewAlert
// ---------------------------------------------------------------------------

/// Input value object used to create a new [`PriceAlert`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewAlert {
    /// Identifier of the user who owns the alert.
    pub owner_id: String,
    /// Raw ticker symbol (will be normalized by [`PriceAlert::new`]).
    pub symbol: String,
    /// Price level at which the alert should fire.
    pub target_price: f64,
    /// Direction of the price move that triggers the alert.
    pub condition: AlertDirection,
    /// Optional explicit expiry timestamp (Unix-epoch ms).
    ///
    /// When `None`, defaults to `now_ms + DEFAULT_TTL_MS`.
    pub expires_at_ms: Option<i64>,
}

// ---------------------------------------------------------------------------
// AlertStoreError
// ---------------------------------------------------------------------------

/// Errors returned by [`AlertStore`] implementations.
#[derive(Debug, Error)]
pub enum AlertStoreError {
    /// The persisted direction string is not `"Above"` or `"Below"`.
    #[error("unknown alert direction: {0}")]
    UnknownDirection(String),
    /// A lower-level storage failure (Postgres, serialization, etc.).
    #[error("storage error: {0}")]
    Storage(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, AlertStoreError>;

// ---------------------------------------------------------------------------
// AlertStore trait
// ---------------------------------------------------------------------------

/// Async persistence interface for [`PriceAlert`] records.
///
/// All methods are `&self` so implementations can use interior mutability
/// (e.g. `Mutex`) or shared connection pools without requiring `&mut self`.
#[async_trait]
pub trait AlertStore: Send + Sync {
    /// Persist a new alert.
    ///
    /// # Errors
    ///
    /// Returns [`AlertStoreError::Storage`] if the underlying write fails.
    async fn insert(&self, alert: &PriceAlert) -> Result<()>;

    /// Return all alerts owned by `owner_id`, newest-first (`created_at_ms`
    /// descending).
    ///
    /// # Errors
    ///
    /// Returns [`AlertStoreError::Storage`] if the underlying read fails.
    async fn list_by_owner(&self, owner_id: &str) -> Result<Vec<PriceAlert>>;

    /// Delete the alert with the given `id`.
    ///
    /// Returns `true` if a record was deleted, `false` if none matched.
    ///
    /// # Errors
    ///
    /// Returns [`AlertStoreError::Storage`] if the underlying write fails.
    async fn delete_by_id(&self, id: &str) -> Result<bool>;

    /// Set `active` on the alert with the given `id`.
    ///
    /// Returns `true` if a record was updated, `false` if none matched.
    ///
    /// # Errors
    ///
    /// Returns [`AlertStoreError::Storage`] if the underlying write fails.
    async fn set_active(&self, id: &str, active: bool) -> Result<bool>;

    /// Return all alerts that are currently eligible to fire: `active`, not
    /// yet `triggered`, and not yet expired (`expires_at_ms > now_ms`).
    ///
    /// # Errors
    ///
    /// Returns [`AlertStoreError::Storage`] if the underlying read fails.
    async fn list_eligible(&self, now_ms: i64) -> Result<Vec<PriceAlert>>;

    /// Atomically mark an alert as fired (`triggered = true`, `active =
    /// false`), but only if it is currently eligible (active, not triggered,
    /// not expired).
    ///
    /// Returns `true` if the transition was applied, `false` if the alert
    /// was not found or was not eligible (fire-once guarantee).
    ///
    /// # Errors
    ///
    /// Returns [`AlertStoreError::Storage`] if the underlying write fails.
    async fn mark_fired(&self, id: &str, now_ms: i64) -> Result<bool>;
}

// ---------------------------------------------------------------------------
// InMemoryAlertStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory implementation of [`AlertStore`].
///
/// Always compiled (no feature flag). Suitable for unit tests and offline
/// scenarios. Uses a `Mutex<Vec<PriceAlert>>` for simplicity.
#[derive(Debug, Default)]
pub struct InMemoryAlertStore {
    records: Mutex<Vec<PriceAlert>>,
}

impl InMemoryAlertStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AlertStore for InMemoryAlertStore {
    async fn insert(&self, alert: &PriceAlert) -> Result<()> {
        self.records
            .lock()
            .map_err(|error| AlertStoreError::Storage(format!("mutex poisoned: {error}")))?
            .push(alert.clone());
        Ok(())
    }

    async fn list_by_owner(&self, owner_id: &str) -> Result<Vec<PriceAlert>> {
        let mut rows: Vec<PriceAlert> = self
            .records
            .lock()
            .map_err(|error| AlertStoreError::Storage(format!("mutex poisoned: {error}")))?
            .iter()
            .filter(|alert| alert.owner_id == owner_id)
            .cloned()
            .collect();
        rows.sort_by_key(|alert| Reverse(alert.created_at_ms));
        Ok(rows)
    }

    async fn delete_by_id(&self, id: &str) -> Result<bool> {
        let mut guard = self
            .records
            .lock()
            .map_err(|error| AlertStoreError::Storage(format!("mutex poisoned: {error}")))?;
        let before = guard.len();
        guard.retain(|alert| alert.id != id);
        Ok(guard.len() < before)
    }

    async fn set_active(&self, id: &str, active: bool) -> Result<bool> {
        let mut guard = self
            .records
            .lock()
            .map_err(|error| AlertStoreError::Storage(format!("mutex poisoned: {error}")))?;
        match guard.iter_mut().find(|alert| alert.id == id) {
            Some(alert) => {
                alert.active = active;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn list_eligible(&self, now_ms: i64) -> Result<Vec<PriceAlert>> {
        let rows: Vec<PriceAlert> = self
            .records
            .lock()
            .map_err(|error| AlertStoreError::Storage(format!("mutex poisoned: {error}")))?
            .iter()
            .filter(|alert| alert.active && !alert.triggered && alert.expires_at_ms > now_ms)
            .cloned()
            .collect();
        Ok(rows)
    }

    async fn mark_fired(&self, id: &str, now_ms: i64) -> Result<bool> {
        let mut guard = self
            .records
            .lock()
            .map_err(|error| AlertStoreError::Storage(format!("mutex poisoned: {error}")))?;
        match guard.iter_mut().find(|alert| alert.id == id) {
            Some(alert) if alert.active && !alert.triggered && alert.expires_at_ms > now_ms => {
                alert.triggered = true;
                alert.active = false;
                alert.updated_at_ms = now_ms;
                Ok(true)
            }
            Some(_) | None => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod pg {
    use async_trait::async_trait;
    use sqlx::Row;
    use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};

    use crate::{AlertDirection, AlertStore, AlertStoreError, PriceAlert, Result};

    /// Postgres-backed implementation of [`AlertStore`].
    ///
    /// Requires the `postgres` feature. Construct via [`PgAlertStore::connect`]
    /// or [`PgAlertStore::from_pool`].
    #[derive(Clone, Debug)]
    pub struct PgAlertStore {
        pool: PgPool,
    }

    impl PgAlertStore {
        /// Open a connection pool against `database_url`.
        ///
        /// # Errors
        ///
        /// Returns [`AlertStoreError::Storage`] if the connection fails.
        pub async fn connect(database_url: &str) -> Result<Self> {
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(database_url)
                .await
                .map_err(|error| AlertStoreError::Storage(format!("postgres connect: {error}")))?;
            Ok(Self { pool })
        }

        /// Adopt a caller-built pool.
        #[must_use]
        pub fn from_pool(pool: PgPool) -> Self {
            Self { pool }
        }

        /// Expose the underlying pool for callers that need raw sqlx access.
        #[must_use]
        pub const fn pool(&self) -> &PgPool {
            &self.pool
        }
    }

    fn row_to_alert(row: &PgRow) -> Result<PriceAlert> {
        let condition: &str = row.get("condition");
        Ok(PriceAlert {
            id: row.get("id"),
            owner_id: row.get("owner_id"),
            symbol: row.get("symbol"),
            target_price: row.get("target_price"),
            condition: condition.parse()?,
            active: row.get("active"),
            triggered: row.get("triggered"),
            expires_at_ms: row.get("expires_at_ms"),
            created_at_ms: row.get("created_at_ms"),
            updated_at_ms: row.get("updated_at_ms"),
        })
    }

    #[async_trait]
    impl AlertStore for PgAlertStore {
        async fn insert(&self, alert: &PriceAlert) -> Result<()> {
            sqlx::query(
                r#"
                insert into system.price_alerts (
                    id, owner_id, symbol, target_price, condition,
                    active, triggered, expires_at_ms, created_at_ms, updated_at_ms
                )
                values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
            )
            .bind(&alert.id)
            .bind(&alert.owner_id)
            .bind(&alert.symbol)
            .bind(alert.target_price)
            .bind(alert.condition.as_str())
            .bind(alert.active)
            .bind(alert.triggered)
            .bind(alert.expires_at_ms)
            .bind(alert.created_at_ms)
            .bind(alert.updated_at_ms)
            .execute(&self.pool)
            .await
            .map_err(|error| AlertStoreError::Storage(format!("insert: {error}")))?;
            Ok(())
        }

        async fn list_by_owner(&self, owner_id: &str) -> Result<Vec<PriceAlert>> {
            let rows = sqlx::query(
                r#"
                select id, owner_id, symbol, target_price, condition,
                       active, triggered, expires_at_ms, created_at_ms, updated_at_ms
                from system.price_alerts
                where owner_id = $1
                order by created_at_ms desc
                "#,
            )
            .bind(owner_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| AlertStoreError::Storage(format!("list_by_owner: {error}")))?;

            rows.iter().map(row_to_alert).collect()
        }

        async fn delete_by_id(&self, id: &str) -> Result<bool> {
            let result = sqlx::query(
                r#"
                delete from system.price_alerts where id = $1
                "#,
            )
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|error| AlertStoreError::Storage(format!("delete_by_id: {error}")))?;

            Ok(result.rows_affected() > 0)
        }

        async fn set_active(&self, id: &str, active: bool) -> Result<bool> {
            let result = sqlx::query(
                r#"
                update system.price_alerts
                set active = $1
                where id = $2
                "#,
            )
            .bind(active)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|error| AlertStoreError::Storage(format!("set_active: {error}")))?;

            Ok(result.rows_affected() > 0)
        }

        async fn list_eligible(&self, now_ms: i64) -> Result<Vec<PriceAlert>> {
            let rows = sqlx::query(
                r#"
                select id, owner_id, symbol, target_price, condition,
                       active, triggered, expires_at_ms, created_at_ms, updated_at_ms
                from system.price_alerts
                where active and not triggered and expires_at_ms > $1
                "#,
            )
            .bind(now_ms)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| AlertStoreError::Storage(format!("list_eligible: {error}")))?;

            rows.iter().map(row_to_alert).collect()
        }

        async fn mark_fired(&self, id: &str, now_ms: i64) -> Result<bool> {
            // Single UPDATE with eligibility predicate in WHERE — atomic
            // fire-once guarantee: only succeeds if the row is currently
            // active, not triggered, and not expired.
            let result = sqlx::query(
                r#"
                update system.price_alerts
                set triggered = true,
                    active = false,
                    updated_at_ms = $1
                where id = $2
                  and active
                  and not triggered
                  and expires_at_ms > $1
                "#,
            )
            .bind(now_ms)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|error| AlertStoreError::Storage(format!("mark_fired: {error}")))?;

            Ok(result.rows_affected() > 0)
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        1_000_000_000_000 // fixed epoch ms for deterministic tests
    }

    fn make_alert(id: &str, owner: &str, symbol: &str, offset_ms: i64) -> PriceAlert {
        let n = now() + offset_ms;
        PriceAlert::new(
            NewAlert {
                owner_id: owner.to_string(),
                symbol: symbol.to_string(),
                target_price: 100.0,
                condition: AlertDirection::Above,
                expires_at_ms: None,
            },
            id.to_string(),
            n,
        )
    }

    // --- PriceAlert::new ---

    #[test]
    fn new_alert_normalizes_symbol() {
        let alert = PriceAlert::new(
            NewAlert {
                owner_id: "u1".to_string(),
                symbol: "  aapl  ".to_string(),
                target_price: 150.0,
                condition: AlertDirection::Above,
                expires_at_ms: None,
            },
            "id1".to_string(),
            now(),
        );
        assert_eq!(alert.symbol, "AAPL");
    }

    #[test]
    fn new_alert_defaults_active_true_triggered_false() {
        let alert = make_alert("id1", "u1", "BTC", 0);
        assert!(alert.active);
        assert!(!alert.triggered);
    }

    #[test]
    fn new_alert_default_expiry_is_90_days() {
        let n = now();
        let alert = PriceAlert::new(
            NewAlert {
                owner_id: "u1".to_string(),
                symbol: "ETH".to_string(),
                target_price: 2000.0,
                condition: AlertDirection::Below,
                expires_at_ms: None,
            },
            "id1".to_string(),
            n,
        );
        assert_eq!(alert.expires_at_ms, n + DEFAULT_TTL_MS);
    }

    #[test]
    fn new_alert_explicit_expiry_respected() {
        let expiry = now() + 1_000;
        let alert = PriceAlert::new(
            NewAlert {
                owner_id: "u1".to_string(),
                symbol: "SP500".to_string(),
                target_price: 5000.0,
                condition: AlertDirection::Above,
                expires_at_ms: Some(expiry),
            },
            "id1".to_string(),
            now(),
        );
        assert_eq!(alert.expires_at_ms, expiry);
    }

    // --- AlertDirection ---

    #[test]
    fn alert_direction_roundtrip_above() {
        let dir: AlertDirection = "Above".parse().expect("parse Above");
        assert_eq!(dir, AlertDirection::Above);
        assert_eq!(dir.as_str(), "Above");
    }

    #[test]
    fn alert_direction_roundtrip_below() {
        let dir: AlertDirection = "Below".parse().expect("parse Below");
        assert_eq!(dir, AlertDirection::Below);
        assert_eq!(dir.as_str(), "Below");
    }

    #[test]
    fn alert_direction_unknown_returns_error() {
        let result: Result<AlertDirection> = "above".parse();
        assert!(result.is_err());
    }

    // --- InMemoryAlertStore ---

    fn store() -> InMemoryAlertStore {
        InMemoryAlertStore::new()
    }

    #[tokio::test]
    async fn insert_and_list_by_owner() {
        let s = store();
        let a1 = make_alert("a1", "user1", "AAPL", 0);
        let a2 = make_alert("a2", "user1", "MSFT", 1_000);
        let a3 = make_alert("a3", "user2", "GOOG", 0);
        s.insert(&a1).await.expect("insert a1");
        s.insert(&a2).await.expect("insert a2");
        s.insert(&a3).await.expect("insert a3");

        let user1 = s.list_by_owner("user1").await.expect("list user1");
        assert_eq!(user1.len(), 2);
        // newest-first: a2 has created_at_ms = now()+1000, a1 = now()
        assert_eq!(user1[0].id, "a2");
        assert_eq!(user1[1].id, "a1");

        let user2 = s.list_by_owner("user2").await.expect("list user2");
        assert_eq!(user2.len(), 1);
        assert_eq!(user2[0].id, "a3");
    }

    #[tokio::test]
    async fn list_by_owner_empty_when_no_records() {
        let s = store();
        let result = s.list_by_owner("nobody").await.expect("list nobody");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn delete_by_id_returns_true_when_found() {
        let s = store();
        let a = make_alert("del1", "u1", "AAPL", 0);
        s.insert(&a).await.expect("insert");
        assert!(s.delete_by_id("del1").await.expect("delete"));
        let list = s.list_by_owner("u1").await.expect("list");
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn delete_by_id_returns_false_when_not_found() {
        let s = store();
        assert!(!s.delete_by_id("ghost").await.expect("delete ghost"));
    }

    #[tokio::test]
    async fn set_active_false_and_back() {
        let s = store();
        let a = make_alert("sa1", "u1", "BTC", 0);
        s.insert(&a).await.expect("insert");

        assert!(s.set_active("sa1", false).await.expect("set false"));
        let list = s.list_by_owner("u1").await.expect("list");
        assert!(!list[0].active);

        assert!(s.set_active("sa1", true).await.expect("set true"));
        let list = s.list_by_owner("u1").await.expect("list");
        assert!(list[0].active);
    }

    #[tokio::test]
    async fn set_active_returns_false_when_not_found() {
        let s = store();
        assert!(!s.set_active("ghost", true).await.expect("set ghost"));
    }

    #[tokio::test]
    async fn list_eligible_excludes_inactive() {
        let s = store();
        let mut a = make_alert("e1", "u1", "ETH", 0);
        a.active = false;
        s.insert(&a).await.expect("insert");
        let eligible = s.list_eligible(now()).await.expect("eligible");
        assert!(eligible.is_empty());
    }

    #[tokio::test]
    async fn list_eligible_excludes_triggered() {
        let s = store();
        let mut a = make_alert("e2", "u1", "ETH", 0);
        a.triggered = true;
        s.insert(&a).await.expect("insert");
        let eligible = s.list_eligible(now()).await.expect("eligible");
        assert!(eligible.is_empty());
    }

    #[tokio::test]
    async fn list_eligible_excludes_expired() {
        let s = store();
        // expires_at_ms = now() + DEFAULT_TTL_MS; query with now_ms = expires_at_ms (not strictly greater)
        let n = now();
        let a = make_alert("e3", "u1", "ETH", 0);
        assert_eq!(a.expires_at_ms, n + DEFAULT_TTL_MS);
        s.insert(&a).await.expect("insert");
        // query at exactly the expiry moment — expires_at_ms > now_ms is false
        let eligible = s.list_eligible(n + DEFAULT_TTL_MS).await.expect("eligible");
        assert!(eligible.is_empty());
    }

    #[tokio::test]
    async fn list_eligible_includes_not_yet_expired() {
        let s = store();
        let a = make_alert("e4", "u1", "ETH", 0);
        s.insert(&a).await.expect("insert");
        let eligible = s.list_eligible(now()).await.expect("eligible");
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].id, "e4");
    }

    #[tokio::test]
    async fn mark_fired_transitions_to_triggered_inactive() {
        let s = store();
        let a = make_alert("f1", "u1", "BTC", 0);
        s.insert(&a).await.expect("insert");
        assert!(s.mark_fired("f1", now()).await.expect("mark_fired"));
        let list = s.list_by_owner("u1").await.expect("list");
        assert!(list[0].triggered);
        assert!(!list[0].active);
    }

    #[tokio::test]
    async fn mark_fired_is_fire_once() {
        let s = store();
        let a = make_alert("f2", "u1", "BTC", 0);
        s.insert(&a).await.expect("insert");
        assert!(s.mark_fired("f2", now()).await.expect("first fire"));
        // second call: alert is now triggered+inactive → not eligible
        assert!(!s.mark_fired("f2", now()).await.expect("second fire"));
    }

    #[tokio::test]
    async fn mark_fired_returns_false_when_not_found() {
        let s = store();
        assert!(!s.mark_fired("ghost", now()).await.expect("ghost fire"));
    }

    #[tokio::test]
    async fn mark_fired_returns_false_when_inactive() {
        let s = store();
        let mut a = make_alert("f3", "u1", "ETH", 0);
        a.active = false;
        s.insert(&a).await.expect("insert");
        assert!(!s.mark_fired("f3", now()).await.expect("inactive fire"));
    }

    #[tokio::test]
    async fn mark_fired_returns_false_when_expired() {
        let s = store();
        let n = now();
        let a = make_alert("f4", "u1", "ETH", 0);
        s.insert(&a).await.expect("insert");
        // fire at expiry time (expires_at_ms == now_ms → not strictly greater)
        assert!(
            !s.mark_fired("f4", n + DEFAULT_TTL_MS)
                .await
                .expect("expired fire")
        );
    }

    #[tokio::test]
    async fn multiple_alerts_same_symbol_different_targets() {
        let s = store();
        let mut a1 = make_alert("m1", "u1", "BTC", 0);
        a1.target_price = 50_000.0;
        let mut a2 = make_alert("m2", "u1", "BTC", 1_000);
        a2.target_price = 60_000.0;
        s.insert(&a1).await.expect("insert a1");
        s.insert(&a2).await.expect("insert a2");
        let list = s.list_by_owner("u1").await.expect("list");
        assert_eq!(list.len(), 2);
    }
}
