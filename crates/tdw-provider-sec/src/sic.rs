//! Embedded Standard Industrial Classification (SIC) code reference.
//!
//! The SEC publishes the SIC code list used by EDGAR as a static reference page
//! (no JSON API). `regulators/sec/sic_search` filters this small, stable table
//! by a query needle, so it is embedded here to keep the route keyless and
//! offline-deterministic. The codes and titles are public reference facts; the
//! office column names the SEC Division of Corporation Finance industry-review
//! office where the SEC assigns one.

/// One SIC reference row: the four-digit code, its industry title, and the SEC
/// industry-review office where one is assigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SicEntry {
    /// Four-digit SIC code.
    pub code: &'static str,
    /// Industry-title description.
    pub description: &'static str,
    /// SEC industry-review office, when assigned.
    pub office: Option<&'static str>,
}

/// A representative slice of the SEC SIC code reference list.
///
/// This covers the commonly-queried industry codes EDGAR filers use. It is not
/// the exhaustive list; callers needing a code absent here can supply it
/// directly to the CIK/submissions routes.
pub const SIC_CODES: &[SicEntry] = &[
    SicEntry {
        code: "0100",
        description: "Agricultural Production - Crops",
        office: Some("Office of Manufacturing"),
    },
    SicEntry {
        code: "1000",
        description: "Metal Mining",
        office: Some("Office of Energy & Transportation"),
    },
    SicEntry {
        code: "1311",
        description: "Crude Petroleum & Natural Gas",
        office: Some("Office of Energy & Transportation"),
    },
    SicEntry {
        code: "1531",
        description: "Operative Builders",
        office: Some("Office of Real Estate & Construction"),
    },
    SicEntry {
        code: "2000",
        description: "Food and Kindred Products",
        office: Some("Office of Manufacturing"),
    },
    SicEntry {
        code: "2834",
        description: "Pharmaceutical Preparations",
        office: Some("Office of Life Sciences"),
    },
    SicEntry {
        code: "2836",
        description: "Biological Products (No Diagnostic Substances)",
        office: Some("Office of Life Sciences"),
    },
    SicEntry {
        code: "3571",
        description: "Electronic Computers",
        office: Some("Office of Technology"),
    },
    SicEntry {
        code: "3576",
        description: "Computer Communications Equipment",
        office: Some("Office of Technology"),
    },
    SicEntry {
        code: "3674",
        description: "Semiconductors & Related Devices",
        office: Some("Office of Manufacturing"),
    },
    SicEntry {
        code: "3711",
        description: "Motor Vehicles & Passenger Car Bodies",
        office: Some("Office of Manufacturing"),
    },
    SicEntry {
        code: "4813",
        description: "Telephone Communications (No Radiotelephone)",
        office: Some("Office of Technology"),
    },
    SicEntry {
        code: "4911",
        description: "Electric Services",
        office: Some("Office of Energy & Transportation"),
    },
    SicEntry {
        code: "5812",
        description: "Eating Places",
        office: Some("Office of Trade & Services"),
    },
    SicEntry {
        code: "6022",
        description: "State Commercial Banks",
        office: Some("Office of Finance"),
    },
    SicEntry {
        code: "6189",
        description: "Asset-Backed Securities",
        office: Some("Office of Structured Finance"),
    },
    SicEntry {
        code: "6199",
        description: "Finance Services",
        office: Some("Office of Finance"),
    },
    SicEntry {
        code: "6770",
        description: "Blank Checks",
        office: Some("Office of Real Estate & Construction"),
    },
    SicEntry {
        code: "7370",
        description: "Services-Computer Programming, Data Processing, Etc.",
        office: Some("Office of Technology"),
    },
    SicEntry {
        code: "7372",
        description: "Services-Prepackaged Software",
        office: Some("Office of Technology"),
    },
    SicEntry {
        code: "8742",
        description: "Services-Management Consulting Services",
        office: Some("Office of Trade & Services"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn codes_are_unique_and_four_digits() {
        let mut seen = BTreeSet::new();
        for entry in SIC_CODES {
            assert!(seen.insert(entry.code), "duplicate SIC code {}", entry.code);
            assert_eq!(entry.code.len(), 4, "SIC code not 4 digits: {}", entry.code);
            assert!(
                entry.code.chars().all(|c| c.is_ascii_digit()),
                "SIC code not numeric: {}",
                entry.code
            );
            assert!(
                !entry.description.is_empty(),
                "SIC code {} has empty description",
                entry.code
            );
        }
    }
}
