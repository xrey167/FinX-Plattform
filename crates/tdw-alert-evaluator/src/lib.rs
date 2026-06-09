//! Price-alert evaluation function — a memoized, resumable [`FunctionDef`] that
//! loads eligible alerts, fetches fresh quotes, evaluates `Above`/`Below`
//! conditions, atomically fires alerts exactly once, and dispatches a
//! notification hook.
//!
//! # Architecture
//!
//! The crate is a thin layer over three domain interfaces:
//!
//! - [`AlertStore`] — persistence for [`PriceAlert`] records (from
//!   `tdw-alerts`).
//! - [`QuoteSource`] — single-symbol quote fetcher; see [`QuoteSource`] for the
//!   production wiring note.
//! - [`AlertNotifier`] — best-effort notification hook; failures are logged and
//!   never surface as run errors because the alert has already been atomically
//!   fired before `notify` is called.
//!
//! # Feature flags
//!
//! | Feature | Effect |
//! |---------|--------|
//! | *(none)* | Core types, [`NoopNotifier`], [`StaticQuoteSource`], function builder — always compiled, no I/O |
//! | `smtp` | Enables [`EmailNotifier`] backed by [`tdw_email::TransactionalMailer`] |
//!
//! # Clock boundary
//!
//! `now_ms` is read **once** at handler entry — the same pattern used by
//! `tdw_functions::job::FunctionJobHandler` (which calls `SystemTime::now()`
//! once per job and passes the value down). The four memoized steps all receive
//! the same `now_ms` so step results are deterministic across a replay.
//! Production callers that drive the registry via the worker bridge inherit the
//! clock boundary from `FunctionJobHandler`; in tests the timestamp is supplied
//! directly to [`FunctionRegistry::invoke`].

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tdw_alerts::{AlertStore, PriceAlert};
use tdw_domain::QuoteSnapshot;
use tdw_functions::{FunctionDef, FunctionError, FunctionRegistry, Trigger};

pub use notifier::{AlertNotifier, NoopNotifier};
pub use quote_source::{QuoteSource, StaticQuoteSource};

#[cfg(feature = "smtp")]
pub use notifier::EmailNotifier;

mod notifier;
mod quote_source;

// ---------------------------------------------------------------------------
// AlertEvalDeps
// ---------------------------------------------------------------------------

/// Dependency bundle passed to [`alert_evaluation_function`].
///
/// All three components are `Arc`-wrapped trait objects so they can be shared
/// cheaply across async tasks and across the registry's `Arc<FunctionDef>`.
#[derive(Clone)]
pub struct AlertEvalDeps {
    /// Source of eligible [`PriceAlert`] records and atomic fire-once logic.
    pub alerts: Arc<dyn AlertStore>,
    /// Quote fetcher called for each distinct symbol in the eligible alert set.
    pub quotes: Arc<dyn QuoteSource>,
    /// Best-effort notification hook called after a successful atomic fire.
    pub notifier: Arc<dyn AlertNotifier>,
}

// ---------------------------------------------------------------------------
// alert_evaluation_function
// ---------------------------------------------------------------------------

/// Build the `"price-alert-evaluation"` [`FunctionDef`].
///
/// The returned definition registers a [`Trigger::Cron`] on `*/5 * * * *`
/// (every five minutes) and implements four memoized steps:
///
/// 1. **`load_eligible`** — list alerts eligible to fire at `now_ms`.
/// 2. **`fetch_quotes`** — fetch a [`QuoteSnapshot`] for each distinct symbol
///    in deterministic (alphabetical) order; per-symbol errors are logged and
///    skipped.
/// 3. **`evaluate`** — apply `Above` / `Below` conditions; collect ids of
///    alerts that should fire.
/// 4. **`fire_and_notify`** — call [`AlertStore::mark_fired`] for each id
///    (atomic fire-once); only on `true` call [`AlertNotifier::notify`]
///    (best-effort, errors logged).
///
/// Returns `{"processed": <eligible count>, "triggered": <actually fired
/// count>}`.
#[must_use]
// Cohesive builder: the entire body is a single `FunctionDef::from_fn` call whose
// async closure threads four `ctx.step(...)` stages, each with its own `move` capture
// of cloned `deps`. The stages share the per-run `now_ms` clock boundary and the
// borrowed `ctx`; lifting any stage into a helper would have to reconstruct that
// closure/capture chain, so it is kept inline to preserve behavior exactly.
#[allow(clippy::too_many_lines)]
pub fn alert_evaluation_function(deps: AlertEvalDeps) -> FunctionDef {
    FunctionDef::from_fn(
        "price-alert-evaluation",
        vec![Trigger::Cron("*/5 * * * *".into())],
        move |ctx, _payload| {
            let deps = deps.clone();
            Box::pin(async move {
                // Clock boundary: read once at handler entry; all steps use
                // this single timestamp so replays remain deterministic.
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));

                // ----------------------------------------------------------
                // Step 1: load eligible alerts
                // ----------------------------------------------------------
                let step1: Value = ctx
                    .step("load_eligible", || {
                        let alerts = Arc::clone(&deps.alerts);
                        async move {
                            let eligible = alerts.list_eligible(now_ms).await.map_err(|err| {
                                FunctionError::Step {
                                    step: "load_eligible".to_string(),
                                    message: err.to_string(),
                                }
                            })?;
                            serde_json::to_value(&eligible)
                                .map_err(|err| FunctionError::Serialization(err.to_string()))
                        }
                    })
                    .await?;

                let eligible: Vec<PriceAlert> = serde_json::from_value(step1).map_err(|err| {
                    FunctionError::Serialization(format!("deserialize load_eligible result: {err}"))
                })?;

                // Short-circuit: nothing to do.
                if eligible.is_empty() {
                    return Ok(json!({"processed": 0, "triggered": 0}));
                }

                let eligible_count = eligible.len();

                // ----------------------------------------------------------
                // Step 2: fetch quotes (deduplicated, alphabetical order)
                // ----------------------------------------------------------
                let step2: Value = ctx
                    .step("fetch_quotes", || {
                        let quotes_src = Arc::clone(&deps.quotes);
                        let alerts_snap = eligible.clone();
                        async move {
                            // Deduplicate symbols in sorted order for
                            // determinism.
                            let mut symbols: Vec<String> = alerts_snap
                                .iter()
                                .map(|a| a.symbol.clone())
                                .collect::<std::collections::BTreeSet<_>>()
                                .into_iter()
                                .collect();
                            symbols.sort();

                            let mut map: BTreeMap<String, QuoteSnapshot> = BTreeMap::new();
                            for sym in &symbols {
                                match quotes_src.quote(sym).await {
                                    Ok(snap) => {
                                        map.insert(sym.clone(), snap);
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            symbol = %sym,
                                            error = %err,
                                            "quote fetch failed — skipping symbol"
                                        );
                                    }
                                }
                            }

                            serde_json::to_value(&map)
                                .map_err(|err| FunctionError::Serialization(err.to_string()))
                        }
                    })
                    .await?;

                let quote_map: BTreeMap<String, QuoteSnapshot> = serde_json::from_value(step2)
                    .map_err(|err| {
                        FunctionError::Serialization(format!(
                            "deserialize fetch_quotes result: {err}"
                        ))
                    })?;

                // ----------------------------------------------------------
                // Step 3: evaluate conditions
                // ----------------------------------------------------------
                let step3: Value = ctx
                    .step("evaluate", || {
                        let alerts_snap = eligible.clone();
                        let quotes_snap = quote_map.clone();
                        async move {
                            let mut fired_ids: Vec<String> = Vec::new();
                            for alert in &alerts_snap {
                                let Some(snap) = quotes_snap.get(&alert.symbol) else {
                                    // No quote available — skip.
                                    continue;
                                };
                                let fires = match alert.condition {
                                    tdw_alerts::AlertDirection::Above => {
                                        snap.current_price >= alert.target_price
                                    }
                                    tdw_alerts::AlertDirection::Below => {
                                        snap.current_price <= alert.target_price
                                    }
                                };
                                if fires {
                                    fired_ids.push(alert.id.clone());
                                }
                            }
                            // Sort for deterministic output.
                            fired_ids.sort();
                            serde_json::to_value(&fired_ids)
                                .map_err(|err| FunctionError::Serialization(err.to_string()))
                        }
                    })
                    .await?;

                let fired_ids: Vec<String> = serde_json::from_value(step3).map_err(|err| {
                    FunctionError::Serialization(format!("deserialize evaluate result: {err}"))
                })?;

                // ----------------------------------------------------------
                // Step 4: fire_and_notify
                // ----------------------------------------------------------
                let step4: Value = ctx
                    .step("fire_and_notify", || {
                        let store = Arc::clone(&deps.alerts);
                        let notifier = Arc::clone(&deps.notifier);
                        let ids_snap = fired_ids.clone();
                        let quotes_snap = quote_map.clone();
                        let alerts_snap = eligible.clone();
                        async move {
                            let mut triggered_count: usize = 0;

                            for id in &ids_snap {
                                // Atomic fire-once: mark_fired returns false if
                                // another run won the race.
                                let fired = store.mark_fired(id, now_ms).await.map_err(|err| {
                                    FunctionError::Step {
                                        step: "fire_and_notify".to_string(),
                                        message: format!("mark_fired({id}): {err}"),
                                    }
                                })?;

                                if !fired {
                                    // Another run fired this alert first — skip
                                    // notification.
                                    tracing::debug!(
                                        alert_id = %id,
                                        "mark_fired returned false — skipping notify (race)"
                                    );
                                    continue;
                                }

                                triggered_count += 1;

                                // Locate the alert and its quote for the
                                // notification payload.
                                if let Some(alert) = alerts_snap.iter().find(|a| a.id == *id)
                                    && let Some(snap) = quotes_snap.get(&alert.symbol)
                                {
                                    // Best-effort: log errors but never
                                    // fail the run — the alert is already
                                    // atomically fired.
                                    if let Err(err) = notifier.notify(alert, snap).await {
                                        tracing::warn!(
                                            alert_id = %id,
                                            error = %err,
                                            "notification failed (alert already fired)"
                                        );
                                    }
                                }
                            }

                            serde_json::to_value(json!({
                                "processed": eligible_count,
                                "triggered": triggered_count,
                            }))
                            .map_err(|err| FunctionError::Serialization(err.to_string()))
                        }
                    })
                    .await?;

                Ok(step4)
            })
        },
    )
}

// ---------------------------------------------------------------------------
// register
// ---------------------------------------------------------------------------

/// Register the price-alert evaluation function into `registry`.
///
/// This is a convenience wrapper around
/// [`FunctionRegistry::register`] + [`alert_evaluation_function`].
///
/// # Errors
///
/// Returns [`FunctionError::Store`] if a function with id
/// `"price-alert-evaluation"` is already registered.
pub fn register(registry: &mut FunctionRegistry, deps: AlertEvalDeps) -> Result<(), FunctionError> {
    registry.register(alert_evaluation_function(deps))
}
