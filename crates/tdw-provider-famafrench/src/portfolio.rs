//! Clean-room catalog, query types, and parsers for the Ken French Data Library
//! portfolio-formation routes (OpenBB-parity **P4W9**).
//!
//! Every fact here is from the Data Library's own public documentation at
//! <https://mba.tuck.dartmouth.edu/pages/faculty/ken.french/data_library.html>:
//! the `ftp/*_CSV.zip` archive names, the inner `.CSV` member names, and the
//! published wide-table layout (a leading date column followed by one column per
//! formed portfolio, region, country, or percentile breakpoint). The library is
//! keyless (a public ftp tree of ZIP-of-CSV files).
//!
//! These routes are distinct from the single research-factor route in
//! [`crate::catalog`]: the portfolio / breakpoint files are *wide* tables whose
//! column set varies by dataset, so they normalize to the long-format
//! [`tdw_domain::PortfolioReturn`] / [`tdw_domain::Breakpoint`] models (one row
//! per `(date, column)` cell) rather than the fixed-field
//! [`tdw_domain::FactorReturn`]. This module is dependency-free (no `http`
//! feature) so the catalog and its parsers compile in the default offline build.

/// `OpenBB`-parity command path for the breakpoints route.
pub const BREAKPOINTS_COMMAND: &str = "economy/factors/famafrench/breakpoints";
/// `OpenBB`-parity command path for the US portfolio-returns route.
pub const US_PORTFOLIO_COMMAND: &str = "economy/factors/famafrench/us_portfolio_returns";
/// `OpenBB`-parity command path for the regional portfolio-returns route.
pub const REGIONAL_PORTFOLIO_COMMAND: &str =
    "economy/factors/famafrench/regional_portfolio_returns";
/// `OpenBB`-parity command path for the country portfolio-returns route.
pub const COUNTRY_PORTFOLIO_COMMAND: &str = "economy/factors/famafrench/country_portfolio_returns";
/// `OpenBB`-parity command path for the international index-returns route.
pub const INTERNATIONAL_INDEX_COMMAND: &str =
    "economy/factors/famafrench/international_index_returns";

/// Whether a parsed route serves percentile breakpoints (which carry through
/// unscaled) or portfolio returns (which the source publishes in percent and the
/// parser converts to a decimal fraction).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortfolioKind {
    /// Portfolio / index returns — source publishes percent, normalize to a
    /// decimal fraction.
    Return,
    /// Percentile / count breakpoints — carry the source levels through
    /// unscaled.
    Breakpoint,
}

/// One concrete Ken French portfolio / breakpoint dataset: the command it backs,
/// the ZIP archive to fetch, the inner CSV member to parse, and whether it is a
/// return or a breakpoint table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortfolioDataset {
    /// `OpenBB`-parity command path this dataset standardizes.
    pub command: &'static str,
    /// ZIP archive file name under the Data Library `ftp/` directory.
    pub zip_file: &'static str,
    /// Inner CSV member name inside the archive.
    pub csv_member: &'static str,
    /// Whether the table holds returns or breakpoints.
    pub kind: PortfolioKind,
    /// Human-readable description of the route.
    pub description: &'static str,
}

/// Every portfolio / breakpoint dataset this provider can resolve, keyed by its
/// `OpenBB`-parity command path. The ZIP/CSV names are public facts from the
/// Data Library `ftp/` tree.
///
/// One representative public dataset backs each route; the leading wide table in
/// the archive (the value-weighted block for the multi-block portfolio files) is
/// the one normalized. This mirrors how the research-factor route resolves a
/// single concrete archive per request.
pub const DATASETS: &[PortfolioDataset] = &[
    PortfolioDataset {
        command: BREAKPOINTS_COMMAND,
        // Size (market-equity) percentile breakpoints, monthly.
        zip_file: "ME_Breakpoints_CSV.zip",
        csv_member: "ME_Breakpoints.CSV",
        kind: PortfolioKind::Breakpoint,
        description: "Ken French market-equity (size) portfolio-formation breakpoints (keyless).",
    },
    PortfolioDataset {
        command: US_PORTFOLIO_COMMAND,
        // US portfolios formed on book-to-market, monthly.
        zip_file: "Portfolios_Formed_on_BE-ME_CSV.zip",
        csv_member: "Portfolios_Formed_on_BE-ME.CSV",
        kind: PortfolioKind::Return,
        description: "Ken French US portfolios formed on book-to-market returns (keyless).",
    },
    PortfolioDataset {
        command: REGIONAL_PORTFOLIO_COMMAND,
        // Developed-markets 6 portfolios formed on size and book-to-market.
        zip_file: "Developed_6_Portfolios_ME_BE-ME_CSV.zip",
        csv_member: "Developed_6_Portfolios_ME_BE-ME.csv",
        kind: PortfolioKind::Return,
        description: "Ken French developed-markets size/value portfolio returns (keyless).",
    },
    PortfolioDataset {
        command: COUNTRY_PORTFOLIO_COMMAND,
        // Japan 6 portfolios formed on size and book-to-market.
        zip_file: "Japan_6_Portfolios_ME_BE-ME_CSV.zip",
        csv_member: "Japan_6_Portfolios_ME_BE-ME.csv",
        kind: PortfolioKind::Return,
        description: "Ken French country (Japan) size/value portfolio returns (keyless).",
    },
    PortfolioDataset {
        command: INTERNATIONAL_INDEX_COMMAND,
        // Developed-markets research-factor index returns.
        zip_file: "Developed_3_Factors_CSV.zip",
        csv_member: "Developed_3_Factors.csv",
        kind: PortfolioKind::Return,
        description: "Ken French developed-markets international index returns (keyless).",
    },
];

/// Resolve the concrete portfolio dataset for an `OpenBB`-parity `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static PortfolioDataset> {
    DATASETS.iter().find(|dataset| dataset.command == command)
}

/// One standardized portfolio / breakpoint endpoint.
///
/// Mirrors the research-factor [`crate::catalog::ENDPOINTS`] shape so the
/// `tdw-service-api` conformance test can pin each catalog route to its dispatch
/// binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortfolioEndpoint {
    /// `OpenBB`-parity command path this endpoint standardizes.
    pub command: &'static str,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
    /// Whether the endpoint serves returns or breakpoints.
    pub kind: PortfolioKind,
}

/// The standardized portfolio / breakpoint endpoint catalog (one entry per
/// route), derived from [`DATASETS`] so the two never drift.
#[must_use]
pub fn endpoints() -> Vec<PortfolioEndpoint> {
    DATASETS
        .iter()
        .map(|dataset| PortfolioEndpoint {
            command: dataset.command,
            description: dataset.description,
            kind: dataset.kind,
        })
        .collect()
}

/// Whether `token` is an all-ASCII-digit date cell (8 digits daily `YYYYMMDD`,
/// 6 digits monthly `YYYYMM`). Mirrors the research-factor parser's date guard.
fn is_date_cell(token: &str) -> bool {
    let token = token.trim();
    matches!(token.len(), 6 | 8) && token.bytes().all(|b| b.is_ascii_digit())
}

/// Normalize a raw Ken French date cell: `YYYYMMDD` → `YYYY-MM-DD`, `YYYYMM` →
/// `YYYY-MM`.
fn normalize_date(cell: &str) -> String {
    let cell = cell.trim();
    if cell.len() == 8 {
        format!("{}-{}-{}", &cell[0..4], &cell[4..6], &cell[6..8])
    } else {
        format!("{}-{}", &cell[0..4], &cell[4..6])
    }
}

/// Parse a numeric cell, treating the library's missing-value sentinels
/// (`-99.99` / `-999`) and blanks as absent. When `as_fraction` is set the value
/// is divided by 100 (the Data Library publishes returns in percent).
fn parse_cell(cell: &str, as_fraction: bool) -> Option<f64> {
    let trimmed = cell.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = trimmed.parse::<f64>().ok()?;
    if (value - -99.99).abs() < 1e-9 || (value - -999.0).abs() < 1e-9 {
        return None;
    }
    Some(if as_fraction { value / 100.0 } else { value })
}

/// One parsed wide-table cell: a `(date, column-label, value)` triple. The
/// caller maps it into the appropriate long-format domain model.
#[derive(Clone, Debug, PartialEq)]
pub struct WideCell {
    /// Normalized observation date.
    pub date: String,
    /// Column label the value came from.
    pub label: String,
    /// Parsed value (already scaled per [`PortfolioKind`]).
    pub value: Option<f64>,
}

/// Parse a Ken French wide portfolio / breakpoint CSV table (the inner `.CSV`
/// member, as text) into long-format `(date, column, value)` cells.
///
/// Skips the descriptive header preamble, reads the first non-empty header line
/// whose own first cell is *not* a date as the column-label row, then reads each
/// subsequent data row (a line whose first cell is an all-digit date). Each wide
/// cell after the leading date column becomes one [`WideCell`]. Parsing stops at
/// the first blank / non-date line after the data block begins, so any appended
/// secondary table (annual factors, a second weighting block) is ignored.
///
/// For [`PortfolioKind::Return`] datasets the percent values are converted to a
/// decimal fraction; for [`PortfolioKind::Breakpoint`] datasets they pass through
/// unscaled. Returns an empty `Vec` when the table carries no recognizable
/// header (the fetcher surfaces that as an empty result, matching the keyless
/// providers' lenient empty-window behavior).
#[must_use]
pub fn parse_wide_table(text: &str, kind: PortfolioKind) -> Vec<WideCell> {
    let as_fraction = matches!(kind, PortfolioKind::Return);
    let mut labels: Option<Vec<String>> = None;
    let mut cells = Vec::new();
    let mut started = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if started {
                break;
            }
            continue;
        }
        let first_cell = trimmed.split(',').next().unwrap_or("").trim();

        if is_date_cell(first_cell) {
            let Some(labels) = labels.as_ref() else {
                // A data row before any header — skip until a header appears.
                continue;
            };
            started = true;
            let date = normalize_date(first_cell);
            for (cell, label) in trimmed.split(',').skip(1).zip(labels.iter().skip(1)) {
                if label.is_empty() {
                    continue;
                }
                cells.push(WideCell {
                    date: date.clone(),
                    label: label.clone(),
                    value: parse_cell(cell, as_fraction),
                });
            }
            continue;
        }

        if started {
            // First non-date line after the data block ends it.
            break;
        }

        // Header preamble. The column-label row is the first line that carries at
        // least one non-empty label past the leading (date) column.
        let candidate: Vec<String> = trimmed.split(',').map(|c| c.trim().to_string()).collect();
        if candidate.iter().skip(1).any(|c| !c.is_empty()) {
            labels = Some(candidate);
        }
    }

    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasets_cover_every_command_uniquely() {
        use std::collections::BTreeSet;
        let mut seen = BTreeSet::new();
        for dataset in DATASETS {
            assert!(
                seen.insert(dataset.command),
                "duplicate command: {}",
                dataset.command
            );
            assert!(dataset.zip_file.to_ascii_uppercase().ends_with("_CSV.ZIP"));
        }
        assert_eq!(DATASETS.len(), 5);
    }

    #[test]
    fn resolve_finds_known_and_misses_unknown() {
        let breakpoints = resolve(BREAKPOINTS_COMMAND).expect("breakpoints resolve");
        assert_eq!(breakpoints.kind, PortfolioKind::Breakpoint);
        let us = resolve(US_PORTFOLIO_COMMAND).expect("us resolve");
        assert_eq!(us.kind, PortfolioKind::Return);
        assert!(resolve("economy/factors/famafrench/bogus").is_none());
    }

    #[test]
    fn endpoints_mirror_datasets() {
        let endpoints = endpoints();
        assert_eq!(endpoints.len(), DATASETS.len());
        for (endpoint, dataset) in endpoints.iter().zip(DATASETS) {
            assert_eq!(endpoint.command, dataset.command);
            assert_eq!(endpoint.kind, dataset.kind);
        }
    }

    const PORTFOLIO_FIXTURE: &str = "\
This file was created by ...

,SMALL LoBM,ME1 BM2,BIG HiBM
192607,1.50,-99.99,0.30
192608,-0.50,0.20,
192609,2.00,1.10,0.40

  Equal Weighted Returns -- Monthly
,SMALL LoBM,ME1 BM2,BIG HiBM
192607,9.99,9.99,9.99
";

    #[test]
    fn parses_portfolio_wide_table_to_long_cells() {
        let cells = parse_wide_table(PORTFOLIO_FIXTURE, PortfolioKind::Return);
        // Three date rows × three portfolio columns = nine cells; the appended
        // equal-weighted block is ignored.
        assert_eq!(cells.len(), 9, "cells={cells:#?}");
        assert_eq!(cells[0].date, "1926-07");
        assert_eq!(cells[0].label, "SMALL LoBM");
        // Percent -> fraction.
        assert!((cells[0].value.expect("value") - 0.015).abs() < 1e-12);
        // -99.99 sentinel -> None.
        assert_eq!(cells[1].value, None);
        // Empty cell -> None.
        assert_eq!(cells[5].value, None, "cells={cells:#?}");
    }

    const BREAKPOINT_FIXTURE: &str = "\
This file was created by ...

192607,5,12.50,95.00
192608,6,13.00,96.00
";

    #[test]
    fn parses_breakpoint_table_unscaled_with_synthetic_header() {
        // Breakpoint files lead with a numeric header-less block; supply a header
        // so the parser has column labels, then assert values pass through
        // unscaled.
        let with_header = format!(",Count,5,95\n{BREAKPOINT_FIXTURE}");
        let cells = parse_wide_table(&with_header, PortfolioKind::Breakpoint);
        assert_eq!(cells.len(), 6, "cells={cells:#?}");
        assert_eq!(cells[0].date, "1926-07");
        assert_eq!(cells[0].label, "Count");
        // Unscaled: 5 stays 5, not 0.05.
        assert!((cells[0].value.expect("count") - 5.0).abs() < 1e-12);
        assert!((cells[1].value.expect("bp5") - 12.5).abs() < 1e-12);
    }
}
