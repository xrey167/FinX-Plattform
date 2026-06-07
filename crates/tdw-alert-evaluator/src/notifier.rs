//! [`AlertNotifier`] trait, [`NoopNotifier`], and the `smtp`-gated
//! [`EmailNotifier`].

use async_trait::async_trait;
use tdw_alerts::PriceAlert;
use tdw_domain::QuoteSnapshot;

// ---------------------------------------------------------------------------
// AlertNotifier
// ---------------------------------------------------------------------------

/// Best-effort notification hook called after a price alert fires.
///
/// Implementations must not panic.  The evaluation function treats any `Err`
/// return as a warning — it logs the error and continues, because the alert
/// has already been atomically marked as fired before `notify` is called.
#[async_trait]
pub trait AlertNotifier: Send + Sync {
    /// Notify the alert owner that `alert` has fired against `quote`.
    ///
    /// # Errors
    ///
    /// Returns a `String` describing the failure.  The caller logs this and
    /// does **not** fail the run.
    async fn notify(&self, alert: &PriceAlert, quote: &QuoteSnapshot) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// NoopNotifier
// ---------------------------------------------------------------------------

/// A no-op [`AlertNotifier`] that always returns `Ok(())`.
///
/// Always compiled.  Use in tests or when notification is intentionally
/// disabled.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopNotifier;

#[async_trait]
impl AlertNotifier for NoopNotifier {
    async fn notify(&self, _alert: &PriceAlert, _quote: &QuoteSnapshot) -> Result<(), String> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// EmailNotifier (smtp feature)
// ---------------------------------------------------------------------------

/// An [`AlertNotifier`] that sends a transactional email via
/// [`tdw_email::TransactionalMailer`].
///
/// # Owner → recipient mapping
///
/// [`PriceAlert::owner_id`] is used directly as the recipient email address.
/// This is a placeholder: the canonical owner→email lookup belongs to
/// `tdw-identity` (rank 13 in the OpenStock adaptation backlog) and is not yet
/// implemented.  Once that crate lands, replace `alert.owner_id` with a
/// resolved address from the identity store.
///
/// Requires the `smtp` feature.
#[cfg(feature = "smtp")]
pub struct EmailNotifier {
    mailer: tdw_email::TransactionalMailer,
}

#[cfg(feature = "smtp")]
impl std::fmt::Debug for EmailNotifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmailNotifier")
            .field("mailer", &"<TransactionalMailer>")
            .finish()
    }
}

#[cfg(feature = "smtp")]
impl EmailNotifier {
    /// Create a notifier from an already-constructed mailer.
    #[must_use]
    pub fn new(mailer: tdw_email::TransactionalMailer) -> Self {
        Self { mailer }
    }
}

#[cfg(feature = "smtp")]
#[async_trait]
impl AlertNotifier for EmailNotifier {
    async fn notify(&self, alert: &PriceAlert, quote: &QuoteSnapshot) -> Result<(), String> {
        use std::collections::BTreeMap;

        let direction = alert.condition.as_str().to_lowercase();
        let mut vars: BTreeMap<&str, String> = BTreeMap::new();
        // `market_news_summary` is the closest generic template; a dedicated
        // `price_alert` template is a follow-up (rank 13).
        vars.insert("name", alert.owner_id.clone());
        vars.insert(
            "summary",
            format!(
                "{} alert fired: {} {} {:.4} (current price: {:.4})",
                direction,
                alert.symbol,
                alert.condition.as_str(),
                alert.target_price,
                quote.current_price,
            ),
        );
        vars.insert("date", quote.ts_ms.to_string());

        let html = tdw_email::render_template("market_news_summary", &vars)
            .map_err(|err| format!("render template: {err}"))?;

        let msg = tdw_email::EmailMessage {
            from: String::new(), // mailer uses its configured from_address
            // DEVIATION: using owner_id as recipient until tdw-identity lands
            to: alert.owner_id.clone(),
            subject: format!(
                "Price alert: {} {} {:.4}",
                alert.symbol,
                alert.condition.as_str(),
                alert.target_price
            ),
            text: format!(
                "Your price alert for {} has fired. Current price: {:.4}",
                alert.symbol, quote.current_price
            ),
            html,
        };

        self.mailer
            .send(&msg)
            .await
            .map_err(|err| format!("smtp send: {err}"))
    }
}
