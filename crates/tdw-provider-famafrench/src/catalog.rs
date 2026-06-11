//! Clean-room catalog mapping the `OpenBB`-parity `economy/factors/famafrench`
//! command onto the Ken French Data Library research-factor ZIP archives
//! (OpenBB-parity item **P2W6**).
//!
//! Every fact here is from the Data Library's own public documentation at
//! <https://mba.tuck.dartmouth.edu/pages/faculty/ken.french/data_library.html>:
//! the `ftp/*_CSV.zip` archive names, the inner `.CSV` member names, and the
//! published column order (`Mkt-RF,SMB,HML[,RMW,CMA],RF` or a single `Mom`
//! column). The library is keyless (a public ftp tree of ZIP-of-CSV files).
//!
//! A single catalog route (`economy/factors/famafrench`) serves the whole
//! library; the requested `factor_set` ∈ {`3factor`,`5factor`,`momentum`} and
//! `frequency` ∈ {`daily`,`monthly`} select one concrete [`FamaFrenchDataset`]
//! at query time. This module is dependency-free (no `http` feature) so the
//! catalog and its coverage tests compile in the default offline workspace
//! build.

/// The `OpenBB`-parity command path this provider standardizes — the single
/// catalog route every dataset combination resolves under.
pub const COMMAND: &str = "economy/factors/famafrench";

/// Which research-factor set a request selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactorSet {
    /// Fama/French 3 research factors (`Mkt-RF`, `SMB`, `HML`, `RF`).
    ThreeFactor,
    /// Fama/French 5 research factors (`Mkt-RF`, `SMB`, `HML`, `RMW`, `CMA`, `RF`).
    FiveFactor,
    /// The momentum factor (`Mom`).
    Momentum,
}

impl FactorSet {
    /// Parse the public `factor_set` parameter token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "3factor" | "3_factor" | "three_factor" => Some(Self::ThreeFactor),
            "5factor" | "5_factor" | "five_factor" => Some(Self::FiveFactor),
            "momentum" | "mom" => Some(Self::Momentum),
            _ => None,
        }
    }

    /// Canonical lowercase token for this factor set.
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::ThreeFactor => "3factor",
            Self::FiveFactor => "5factor",
            Self::Momentum => "momentum",
        }
    }
}

/// Observation frequency a request selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    /// Daily observations (`YYYYMMDD` dates).
    Daily,
    /// Monthly observations (`YYYYMM` dates).
    Monthly,
}

impl Frequency {
    /// Parse the public `frequency` parameter token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "daily" | "d" => Some(Self::Daily),
            "monthly" | "m" => Some(Self::Monthly),
            _ => None,
        }
    }

    /// Canonical lowercase token for this frequency.
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Monthly => "monthly",
        }
    }
}

/// One concrete Ken French archive: the ZIP file to fetch and the CSV member to
/// parse inside it, for a given `(factor_set, frequency)` selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FamaFrenchDataset {
    /// The selected factor set.
    pub factor_set: FactorSet,
    /// The selected observation frequency.
    pub frequency: Frequency,
    /// ZIP archive file name under the Data Library `ftp/` directory, e.g.
    /// `"F-F_Research_Data_Factors_daily_CSV.zip"`.
    pub zip_file: &'static str,
    /// Inner CSV member name inside the archive, e.g.
    /// `"F-F_Research_Data_Factors_daily.CSV"`.
    pub csv_member: &'static str,
}

/// Every concrete Ken French dataset this provider can resolve, keyed by its
/// `(factor_set, frequency)` selection. The ZIP/CSV names are public facts from
/// the Data Library `ftp/` tree.
pub const DATASETS: &[FamaFrenchDataset] = &[
    FamaFrenchDataset {
        factor_set: FactorSet::ThreeFactor,
        frequency: Frequency::Daily,
        zip_file: "F-F_Research_Data_Factors_daily_CSV.zip",
        csv_member: "F-F_Research_Data_Factors_daily.CSV",
    },
    FamaFrenchDataset {
        factor_set: FactorSet::ThreeFactor,
        frequency: Frequency::Monthly,
        zip_file: "F-F_Research_Data_Factors_CSV.zip",
        csv_member: "F-F_Research_Data_Factors.CSV",
    },
    FamaFrenchDataset {
        factor_set: FactorSet::FiveFactor,
        frequency: Frequency::Daily,
        zip_file: "F-F_Research_Data_5_Factors_2x3_daily_CSV.zip",
        csv_member: "F-F_Research_Data_5_Factors_2x3_daily.CSV",
    },
    FamaFrenchDataset {
        factor_set: FactorSet::FiveFactor,
        frequency: Frequency::Monthly,
        zip_file: "F-F_Research_Data_5_Factors_2x3_CSV.zip",
        csv_member: "F-F_Research_Data_5_Factors_2x3.CSV",
    },
    FamaFrenchDataset {
        factor_set: FactorSet::Momentum,
        frequency: Frequency::Daily,
        zip_file: "F-F_Momentum_Factor_daily_CSV.zip",
        csv_member: "F-F_Momentum_Factor_daily.CSV",
    },
    FamaFrenchDataset {
        factor_set: FactorSet::Momentum,
        frequency: Frequency::Monthly,
        zip_file: "F-F_Momentum_Factor_CSV.zip",
        csv_member: "F-F_Momentum_Factor.CSV",
    },
];

/// Resolve the concrete dataset for a `(factor_set, frequency)` selection.
#[must_use]
pub fn resolve(factor_set: FactorSet, frequency: Frequency) -> Option<&'static FamaFrenchDataset> {
    DATASETS
        .iter()
        .find(|d| d.factor_set == factor_set && d.frequency == frequency)
}

/// One standardized Ken French endpoint.
///
/// Mirrors the sibling-provider `ENDPOINTS` shape so the `tdw-service-api`
/// conformance test can pin the catalog route, the dispatch tables, and this
/// table to one command path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FamaFrenchEndpoint {
    /// `OpenBB`-parity command path this endpoint standardizes.
    pub command: &'static str,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// The standardized Ken French endpoint catalog (one route serves the library).
pub const ENDPOINTS: &[FamaFrenchEndpoint] = &[FamaFrenchEndpoint {
    command: COMMAND,
    description: "Ken French Data Library research factors (3-factor / 5-factor / momentum, \
                  daily or monthly).",
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factor_set_and_frequency_round_trip_tokens() {
        for set in [
            FactorSet::ThreeFactor,
            FactorSet::FiveFactor,
            FactorSet::Momentum,
        ] {
            assert_eq!(FactorSet::parse(set.as_token()), Some(set));
        }
        for freq in [Frequency::Daily, Frequency::Monthly] {
            assert_eq!(Frequency::parse(freq.as_token()), Some(freq));
        }
        assert_eq!(FactorSet::parse("MOMENTUM"), Some(FactorSet::Momentum));
        assert_eq!(Frequency::parse(" Daily "), Some(Frequency::Daily));
        assert!(FactorSet::parse("4factor").is_none());
        assert!(Frequency::parse("weekly").is_none());
    }

    #[test]
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    // Deliberate: pins the literal "_CSV.zip"/".CSV" casing of Ken French
    // archive member names — these are zip-internal strings, not fs paths.
    fn resolve_covers_every_combination_uniquely() {
        for set in [
            FactorSet::ThreeFactor,
            FactorSet::FiveFactor,
            FactorSet::Momentum,
        ] {
            for freq in [Frequency::Daily, Frequency::Monthly] {
                let dataset = resolve(set, freq).expect("every combination resolves");
                assert!(dataset.zip_file.ends_with("_CSV.zip"));
                assert!(dataset.csv_member.ends_with(".CSV"));
            }
        }
        assert_eq!(DATASETS.len(), 6);
    }

    #[test]
    fn endpoints_carry_the_single_command_route() {
        assert_eq!(ENDPOINTS.len(), 1);
        assert_eq!(ENDPOINTS[0].command, COMMAND);
        assert_eq!(COMMAND, "economy/factors/famafrench");
    }
}
