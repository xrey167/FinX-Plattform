//! Clean-room catalog mapping `OpenBB`-style command paths to the public
//! `Intrinio` REST endpoint that backs them (`OpenBB`-parity total wave G002).
//!
//! Each [`IntrinioEndpoint`] standardizes one command path onto an `Intrinio`
//! v2 REST route. Every route template, query parameter, and response field used
//! here is a public fact from `Intrinio`'s OWN API docs at
//! <https://docs.intrinio.com> / <https://api-v2.intrinio.com> (clean-room: not
//! derived from any other vendor's source). The API is keyed: every live request
//! carries the PAID `api_key` query parameter (read from `INTRINIO_API_KEY`).
//!
//! This module is dependency-free (no `http` feature) so the catalog and its
//! coverage tests compile in the default offline workspace build.

/// Which standardized [`tdw_domain`](https://docs.rs) model an `Intrinio`
/// endpoint emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntrinioModel {
    /// Standardizes to [`tdw_domain::CompanyAttribute`] (data-point attributes).
    CompanyAttribute,
    /// Standardizes to [`tdw_domain::FinancialStatement`] (reported financials).
    FinancialStatement,
    /// Standardizes to [`tdw_domain::Estimate`] (analyst forward estimates).
    Estimate,
    /// Standardizes to [`tdw_domain::OptionContract`] (options data / chain).
    OptionContract,
}

/// One standardized `Intrinio` endpoint.
///
/// `command` is the `OpenBB`-style command path it standardizes. `endpoint` is
/// the short dispatch key (the command's `'/'→'_'` form). `description` is a
/// human-readable summary; the concrete `Intrinio` route template lives in the
/// per-route fetcher in `http_fetcher`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntrinioEndpoint {
    /// `OpenBB`-style command path this endpoint standardizes.
    pub command: &'static str,
    /// Short dispatch endpoint key (the command's `'/'→'_'` form).
    pub endpoint: &'static str,
    /// Which standardized domain model the fetcher emits.
    pub model: IntrinioModel,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// The standardized `Intrinio` endpoint catalog.
pub const ENDPOINTS: &[IntrinioEndpoint] = &[
    IntrinioEndpoint {
        command: "equity/fundamental/historical_attributes",
        endpoint: "equity_fundamental_historical_attributes",
        model: IntrinioModel::CompanyAttribute,
        description: "Historical time series for a standardized data-point tag of \
                      a company (Intrinio `/historical_data/{identifier}/{tag}`).",
    },
    IntrinioEndpoint {
        command: "equity/fundamental/latest_attributes",
        endpoint: "equity_fundamental_latest_attributes",
        model: IntrinioModel::CompanyAttribute,
        description: "Latest value of a standardized data-point tag for a company \
                      (Intrinio `/companies/{identifier}/data_point/{tag}`).",
    },
    IntrinioEndpoint {
        command: "equity/fundamental/search_attributes",
        endpoint: "equity_fundamental_search_attributes",
        model: IntrinioModel::CompanyAttribute,
        description: "Search the standardized data-point tag dictionary by query \
                      (Intrinio `/data_tags/search`).",
    },
    IntrinioEndpoint {
        command: "equity/fundamental/reported_financials",
        endpoint: "equity_fundamental_reported_financials",
        model: IntrinioModel::FinancialStatement,
        description: "As-reported financial statement line items for a fundamental \
                      (Intrinio `/fundamentals/{id}/reported_financials`).",
    },
    IntrinioEndpoint {
        command: "equity/estimates/forward_pe",
        endpoint: "equity_estimates_forward_pe",
        model: IntrinioModel::Estimate,
        description: "Forward price-to-earnings analyst estimates for a company \
                      (Intrinio zacks forward-PE estimates).",
    },
    IntrinioEndpoint {
        command: "equity/estimates/forward_sales",
        endpoint: "equity_estimates_forward_sales",
        model: IntrinioModel::Estimate,
        description: "Forward sales analyst estimates for a company (Intrinio \
                      zacks forward-sales estimates).",
    },
    IntrinioEndpoint {
        command: "derivatives/options/unusual",
        endpoint: "derivatives_options_unusual",
        model: IntrinioModel::OptionContract,
        description: "Unusual options activity for a symbol (Intrinio \
                      `/options/unusual_activity/{symbol}`).",
    },
    IntrinioEndpoint {
        command: "derivatives/options/snapshots",
        endpoint: "derivatives_options_snapshots",
        model: IntrinioModel::OptionContract,
        description: "Options market snapshots across the chain (Intrinio \
                      `/options/snapshots`).",
    },
    IntrinioEndpoint {
        command: "derivatives/options/surface",
        endpoint: "derivatives_options_surface",
        model: IntrinioModel::OptionContract,
        description: "Implied-volatility surface inputs over the options chain \
                      (Intrinio `/options/chain/{symbol}/{expiration}`); the \
                      surface solver is a documented follow-up.",
    },
];

/// Resolve a catalog entry by its `OpenBB`-style `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static IntrinioEndpoint> {
    ENDPOINTS.iter().find(|entry| entry.command == command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn command_paths_are_unique() {
        let mut seen = BTreeSet::new();
        for entry in ENDPOINTS {
            assert!(
                seen.insert(entry.command),
                "duplicate command path: {}",
                entry.command
            );
        }
    }

    #[test]
    fn endpoint_keys_are_unique_and_match_command_slug() {
        let mut seen = BTreeSet::new();
        for entry in ENDPOINTS {
            assert!(
                seen.insert(entry.endpoint),
                "duplicate endpoint key: {}",
                entry.endpoint
            );
            assert_eq!(
                entry.endpoint,
                entry.command.replace('/', "_"),
                "endpoint key must be the command's '/'→'_' form",
            );
        }
    }

    #[test]
    fn resolve_finds_known_and_misses_unknown() {
        let unusual = resolve("derivatives/options/unusual").expect("unusual resolve");
        assert_eq!(unusual.model, IntrinioModel::OptionContract);
        let pe = resolve("equity/estimates/forward_pe").expect("forward_pe resolve");
        assert_eq!(pe.model, IntrinioModel::Estimate);
        assert!(resolve("equity/fundamental/bogus").is_none());
    }
}
