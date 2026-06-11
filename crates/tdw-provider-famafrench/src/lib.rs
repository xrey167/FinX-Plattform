#![forbid(unsafe_code)]
//! Offline types, the research-factor CSV parser, and validation for the Ken
//! French Data Library provider (OpenBB-parity item **P2W6**).
//!
//! The real HTTP fetcher lives in [`http_fetcher`] and is only compiled when the
//! `http` feature is enabled. Everything in this module — including the
//! header-skipping factor-table parser ([`parse_factor_table`]) — works without
//! any network access. The Data Library is keyless (a public ftp tree of
//! ZIP-of-CSV archives).
//!
//! The published CSV layout (per the Data Library docs) is several descriptive
//! header lines, then a comma-separated table whose data rows each start with an
//! all-digit date (`YYYYMMDD` daily / `YYYYMM` monthly) followed by the factor
//! columns *in percent*. The parser skips the header preamble, reads the column
//! header to learn which factors the file carries, parses the date rows, stops
//! at the first blank / non-date line (so an appended annual table is ignored),
//! and converts each percent value to a decimal fraction.

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::FamaFrenchHttpFetcher;

pub use catalog::{
    COMMAND, DATASETS, ENDPOINTS, FactorSet, FamaFrenchDataset, FamaFrenchEndpoint, Frequency,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_domain::FactorReturn;
use thiserror::Error;

/// Public base URL for the Ken French Data Library ftp tree.
pub const BASE_URL: &str = "https://mba.tuck.dartmouth.edu/pages/faculty/ken.french/ftp";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "famafrench";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, FamaFrenchProviderError>;

/// Query for the standardized Ken French research-factor route.
///
/// The caller supplies a `factor_set` ∈ {`3factor`,`5factor`,`momentum`} and a
/// `frequency` ∈ {`daily`,`monthly`}; the query resolves them against
/// [`catalog`] to a concrete [`FamaFrenchDataset`] (the ZIP archive + inner CSV
/// member). The Data Library exposes no server-side date/limit filtering, so the
/// shared params are not carried here — callers post-filter the returned rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FamaFrenchQuery {
    /// The requested factor set, canonicalized (`3factor` / `5factor` / `momentum`).
    pub factor_set: String,
    /// The requested observation frequency, canonicalized (`daily` / `monthly`).
    pub frequency: String,
}

impl FamaFrenchQuery {
    /// Build a query from an explicit factor set and frequency.
    ///
    /// # Errors
    ///
    /// Returns [`FamaFrenchProviderError::UnknownDataset`] when the
    /// `(factor_set, frequency)` pair does not resolve to a known dataset.
    pub fn new(factor_set: FactorSet, frequency: Frequency) -> Result<Self> {
        // Resolution is total over the enum domain, but keep the guard so the
        // error path stays identical to `from_value`.
        catalog::resolve(factor_set, frequency).ok_or_else(|| {
            FamaFrenchProviderError::UnknownDataset {
                factor_set: factor_set.as_token().to_string(),
                frequency: frequency.as_token().to_string(),
            }
        })?;
        Ok(Self {
            factor_set: factor_set.as_token().to_string(),
            frequency: frequency.as_token().to_string(),
        })
    }

    /// Build a query from a raw caller payload, reading and validating the
    /// `factor_set` and `frequency` tokens. Both default when absent: a missing
    /// `factor_set` is `3factor`, a missing `frequency` is `daily`.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::InvalidQuery`] when `factor_set` or `frequency`
    /// is present but not a recognized token.
    pub fn from_value(params: &serde_json::Value) -> std::result::Result<Self, tdw_core::Error> {
        let factor_set = match params.get("factor_set") {
            None | Some(serde_json::Value::Null) => FactorSet::ThreeFactor,
            Some(serde_json::Value::String(token)) => FactorSet::parse(token).ok_or_else(|| {
                tdw_core::Error::InvalidQuery(format!("famafrench factor_set {token:?} is unknown"))
            })?,
            Some(_) => {
                return Err(tdw_core::Error::InvalidQuery(
                    "famafrench factor_set must be a string".to_string(),
                ));
            }
        };
        let frequency = match params.get("frequency") {
            None | Some(serde_json::Value::Null) => Frequency::Daily,
            Some(serde_json::Value::String(token)) => Frequency::parse(token).ok_or_else(|| {
                tdw_core::Error::InvalidQuery(format!("famafrench frequency {token:?} is unknown"))
            })?,
            Some(_) => {
                return Err(tdw_core::Error::InvalidQuery(
                    "famafrench frequency must be a string".to_string(),
                ));
            }
        };
        Self::new(factor_set, frequency).map_err(|e| tdw_core::Error::InvalidQuery(e.to_string()))
    }

    /// The selected factor set.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`FamaFrenchQuery::new`] or
    /// [`FamaFrenchQuery::from_value`], since both store a canonical token.
    #[must_use]
    pub fn parsed_factor_set(&self) -> FactorSet {
        FactorSet::parse(&self.factor_set)
            .expect("FamaFrenchQuery::factor_set is canonicalized at construction")
    }

    /// The selected observation frequency.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`FamaFrenchQuery::new`] or
    /// [`FamaFrenchQuery::from_value`], since both store a canonical token.
    #[must_use]
    pub fn parsed_frequency(&self) -> Frequency {
        Frequency::parse(&self.frequency)
            .expect("FamaFrenchQuery::frequency is canonicalized at construction")
    }

    /// The concrete Ken French dataset (ZIP + inner CSV) this query resolves to.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via the public constructors.
    #[must_use]
    pub fn dataset(&self) -> &'static FamaFrenchDataset {
        catalog::resolve(self.parsed_factor_set(), self.parsed_frequency())
            .expect("FamaFrenchQuery resolves to a known dataset at construction")
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FamaFrenchProviderError {
    /// The `(factor_set, frequency)` selection is not a known dataset.
    #[error("famafrench dataset (factor_set={factor_set:?}, frequency={frequency:?}) is unknown")]
    UnknownDataset {
        factor_set: String,
        frequency: String,
    },
    /// The CSV table carried no recognizable column header row.
    #[error("famafrench CSV has no recognizable factor header row")]
    MissingHeader,
}

/// Map a Ken French column label (e.g. `"Mkt-RF"`, `"SMB"`, `"Mom"`) to the
/// [`FactorReturn`] field it populates. Returns `None` for the leading date
/// column and any unrecognized label.
fn column_field(label: &str) -> Option<FactorField> {
    match label.trim().to_ascii_uppercase().as_str() {
        "MKT-RF" => Some(FactorField::MktRf),
        "SMB" => Some(FactorField::Smb),
        "HML" => Some(FactorField::Hml),
        "RMW" => Some(FactorField::Rmw),
        "CMA" => Some(FactorField::Cma),
        "MOM" => Some(FactorField::Mom),
        "RF" => Some(FactorField::Rf),
        _ => None,
    }
}

/// Which [`FactorReturn`] field a parsed CSV column feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FactorField {
    MktRf,
    Smb,
    Hml,
    Rmw,
    Cma,
    Mom,
    Rf,
}

/// Whether `token` is an all-ASCII-digit date cell (8 digits daily `YYYYMMDD`,
/// 6 digits monthly `YYYYMM`). The Ken French data rows are exactly the lines
/// whose first cell is such a date; everything else (preamble, blank lines, the
/// appended annual table header) is skipped.
fn is_date_cell(token: &str) -> bool {
    let token = token.trim();
    matches!(token.len(), 6 | 8) && token.bytes().all(|b| b.is_ascii_digit())
}

/// Normalize a raw Ken French date cell to a calendar date string: `YYYYMMDD`
/// becomes `YYYY-MM-DD`, `YYYYMM` becomes `YYYY-MM`.
fn normalize_date(cell: &str) -> String {
    let cell = cell.trim();
    if cell.len() == 8 {
        format!("{}-{}-{}", &cell[0..4], &cell[4..6], &cell[6..8])
    } else {
        format!("{}-{}", &cell[0..4], &cell[4..6])
    }
}

/// Parse a single percent-valued factor cell into a decimal fraction. The Data
/// Library publishes factors in percent, so `1.23` becomes `0.0123`. Empty
/// cells and the library's missing-value sentinels (`-99.99` / `-999`) yield
/// `None`.
fn parse_percent(cell: &str) -> Option<f64> {
    let trimmed = cell.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = trimmed.parse::<f64>().ok()?;
    // Ken French missing-value sentinels.
    if (value - -99.99).abs() < 1e-9 || (value - -999.0).abs() < 1e-9 {
        return None;
    }
    Some(value / 100.0)
}

/// Parse a Ken French research-factor CSV table (the inner `.CSV` member, as
/// text) into standardized [`FactorReturn`] rows.
///
/// Skips the descriptive header preamble, reads the first column-header line
/// (the line that names the factor columns) to learn the column order, then
/// reads each subsequent data row (a line whose first cell is an all-digit
/// date), converting each percent value to a decimal fraction. Parsing stops at
/// the first blank or non-date line after the data block begins, so an appended
/// annual table is ignored.
///
/// # Errors
///
/// Returns [`FamaFrenchProviderError::MissingHeader`] when no factor-column
/// header row precedes the date rows.
pub fn parse_factor_table(text: &str) -> Result<Vec<FactorReturn>> {
    let mut columns: Option<Vec<Option<FactorField>>> = None;
    let mut rows = Vec::new();
    let mut started = false;

    for line in text.lines() {
        let trimmed = line.trim();
        let first_cell = trimmed.split(',').next().unwrap_or("").trim();

        if is_date_cell(first_cell) {
            // A data row. We must have already seen a column header.
            let Some(cols) = columns.as_ref() else {
                return Err(FamaFrenchProviderError::MissingHeader);
            };
            started = true;
            let mut record = FactorReturn {
                date: normalize_date(first_cell),
                mkt_rf: None,
                smb: None,
                hml: None,
                rmw: None,
                cma: None,
                mom: None,
                rf: None,
            };
            // The leading column is the date; factor columns follow.
            for (cell, field) in trimmed.split(',').skip(1).zip(cols.iter().skip(1)) {
                if let Some(field) = field {
                    let value = parse_percent(cell);
                    match field {
                        FactorField::MktRf => record.mkt_rf = value,
                        FactorField::Smb => record.smb = value,
                        FactorField::Hml => record.hml = value,
                        FactorField::Rmw => record.rmw = value,
                        FactorField::Cma => record.cma = value,
                        FactorField::Mom => record.mom = value,
                        FactorField::Rf => record.rf = value,
                    }
                }
            }
            rows.push(record);
            continue;
        }

        if started {
            // First blank / non-date line after the data block ends it — this is
            // where the appended annual table (if any) begins.
            break;
        }

        // Header preamble. The column-header row is the first line that names at
        // least one recognized factor column.
        let candidate: Vec<Option<FactorField>> = trimmed.split(',').map(column_field).collect();
        if candidate.iter().any(Option::is_some) {
            columns = Some(candidate);
        }
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_resolves_factor_set_and_frequency() {
        let query = FamaFrenchQuery::new(FactorSet::ThreeFactor, Frequency::Daily)
            .unwrap_or_else(|error| panic!("3factor daily query should build: {error}"));
        assert_eq!(query.factor_set, "3factor");
        assert_eq!(query.frequency, "daily");
        assert_eq!(
            query.dataset().zip_file,
            "F-F_Research_Data_Factors_daily_CSV.zip"
        );
    }

    #[test]
    fn from_value_defaults_and_validates() {
        let defaulted = FamaFrenchQuery::from_value(&serde_json::json!({}))
            .unwrap_or_else(|error| panic!("empty payload should default: {error}"));
        assert_eq!(defaulted.factor_set, "3factor");
        assert_eq!(defaulted.frequency, "daily");

        let explicit = FamaFrenchQuery::from_value(&serde_json::json!({
            "factor_set": "5factor",
            "frequency": "monthly"
        }))
        .unwrap_or_else(|error| panic!("explicit payload should build: {error}"));
        assert_eq!(explicit.parsed_factor_set(), FactorSet::FiveFactor);
        assert_eq!(explicit.parsed_frequency(), Frequency::Monthly);
        assert_eq!(
            explicit.dataset().csv_member,
            "F-F_Research_Data_5_Factors_2x3.CSV"
        );

        assert!(
            FamaFrenchQuery::from_value(&serde_json::json!({ "factor_set": "4factor" })).is_err()
        );
        assert!(
            FamaFrenchQuery::from_value(&serde_json::json!({ "frequency": "weekly" })).is_err()
        );
        assert!(FamaFrenchQuery::from_value(&serde_json::json!({ "factor_set": 3 })).is_err());
    }

    /// The exact Ken French 3-factor daily layout: several descriptive header
    /// lines, the factor column header, a few date rows in percent, a blank
    /// line, then an appended annual table that MUST be ignored.
    const THREE_FACTOR_DAILY_FIXTURE: &str = "\
This file was created by CMPT_ME_BEME_RETS using the 202404 CRSP database.
The 1-month TBill return is from Ibbotson and Associates, Inc.

,Mkt-RF,SMB,HML,RF
20240603,0.55,-0.21,0.13,0.022
20240604,-0.34,0.10,-0.05,0.022
20240605,1.20,0.40,-0.10,0.022

Annual Factors: January-December
,Mkt-RF,SMB,HML,RF
2023,21.00,-3.00,-9.00,5.00
";

    #[test]
    fn parses_three_factor_daily_table_skipping_header_and_annual() {
        let rows = parse_factor_table(THREE_FACTOR_DAILY_FIXTURE)
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));

        // Only the three daily rows — the appended annual table is ignored.
        assert_eq!(rows.len(), 3, "rows={rows:#?}");

        assert_eq!(rows[0].date, "2024-06-03");
        // Percent -> fraction: 0.55% -> 0.0055.
        assert!((rows[0].mkt_rf.expect("mkt_rf") - 0.0055).abs() < 1e-12);
        assert!((rows[0].smb.expect("smb") - -0.0021).abs() < 1e-12);
        assert!((rows[0].hml.expect("hml") - 0.0013).abs() < 1e-12);
        assert!((rows[0].rf.expect("rf") - 0.00022).abs() < 1e-12);
        // The 3-factor dataset carries none of the 5-factor / momentum columns.
        assert_eq!(rows[0].rmw, None);
        assert_eq!(rows[0].cma, None);
        assert_eq!(rows[0].mom, None);

        assert_eq!(rows[2].date, "2024-06-05");
        assert!((rows[2].mkt_rf.expect("mkt_rf") - 0.012).abs() < 1e-12);
    }

    #[test]
    fn parses_momentum_single_column_and_monthly_dates() {
        let fixture = "\
This file was created by ...

,Mom
202401,2.50
202402,-1.00
";
        let rows = parse_factor_table(fixture)
            .unwrap_or_else(|error| panic!("momentum fixture should parse: {error}"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, "2024-01");
        assert!((rows[0].mom.expect("mom") - 0.025).abs() < 1e-12);
        assert_eq!(rows[0].mkt_rf, None);
        assert_eq!(rows[1].date, "2024-02");
        assert!((rows[1].mom.expect("mom") - -0.01).abs() < 1e-12);
    }

    #[test]
    fn missing_value_sentinels_become_none() {
        let fixture = "\
,Mkt-RF,SMB,HML,RF
20240603,-99.99,0.10,,0.022
";
        let rows = parse_factor_table(fixture)
            .unwrap_or_else(|error| panic!("sentinel fixture should parse: {error}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].mkt_rf, None, "-99.99 sentinel -> None");
        assert!((rows[0].smb.expect("smb") - 0.001).abs() < 1e-12);
        assert_eq!(rows[0].hml, None, "empty cell -> None");
    }

    #[test]
    fn date_rows_before_any_header_are_an_error() {
        // A malformed file whose data rows are not preceded by a column header.
        let fixture = "\
some preamble with no factor names
20240603,0.55,-0.21,0.13,0.022
";
        let err = parse_factor_table(fixture).expect_err("date rows with no header must error");
        assert_eq!(err, FamaFrenchProviderError::MissingHeader);
    }
}
