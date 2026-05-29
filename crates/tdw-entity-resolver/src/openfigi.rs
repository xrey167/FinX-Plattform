//! Pure (offline) parser for `OpenFIGI` mapping responses.
//!
//! The `OpenFIGI` `POST /v3/mapping` endpoint answers a batch request with a JSON
//! array, one element per mapping job. Each successful element is an object with
//! a `data` array of instrument records; a failed element carries a `warning` or
//! `error` string instead. We only model the fields the seeder needs (FIGI,
//! name, ticker, exchange code) and ignore the rest.
//!
//! This module does NO network I/O on purpose -- the live HTTP fetch lives in
//! the ingest/seeder layer; here we only turn a response body into typed rows so
//! the parsing is unit-testable against fixtures.

use serde::{Deserialize, Serialize};

/// One instrument row extracted from an `OpenFIGI` mapping `data` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenFigiMapping {
    pub figi: String,
    pub name: Option<String>,
    pub ticker: Option<String>,
    pub exch_code: Option<String>,
}

/// Error returned when an `OpenFIGI` mapping response cannot be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenFigiParseError {
    /// The body was not valid JSON.
    InvalidJson(String),
    /// The top level was not the expected array of mapping jobs.
    UnexpectedShape,
}

impl core::fmt::Display for OpenFigiParseError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidJson(message) => write!(formatter, "invalid openfigi json: {message}"),
            Self::UnexpectedShape => {
                write!(formatter, "openfigi response was not an array of jobs")
            }
        }
    }
}

impl std::error::Error for OpenFigiParseError {}

/// Parse an `OpenFIGI` mapping response body into the instrument rows it contains.
///
/// Job elements that carry a `warning`/`error` (no `data`) are skipped. Within a
/// job's `data` array, entries missing a `figi` are skipped (a FIGI is the
/// minimum useful payload).
///
/// # Errors
///
/// Returns [`OpenFigiParseError::InvalidJson`] if the body is not valid JSON, or
/// [`OpenFigiParseError::UnexpectedShape`] if the top level is not a JSON array.
pub fn parse_openfigi_mapping(json: &str) -> Result<Vec<OpenFigiMapping>, OpenFigiParseError> {
    let root: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| OpenFigiParseError::InvalidJson(error.to_string()))?;
    let jobs = root.as_array().ok_or(OpenFigiParseError::UnexpectedShape)?;

    let mut rows = Vec::new();
    for job in jobs {
        let Some(data) = job.get("data").and_then(serde_json::Value::as_array) else {
            // Job failed (warning/error) or had no data -- skip it.
            continue;
        };
        for entry in data {
            let Some(figi) = entry.get("figi").and_then(serde_json::Value::as_str) else {
                continue;
            };
            rows.push(OpenFigiMapping {
                figi: figi.to_string(),
                name: string_field(entry, "name"),
                ticker: string_field(entry, "ticker"),
                exch_code: string_field(entry, "exchCode"),
            });
        }
    }
    Ok(rows)
}

fn string_field(entry: &serde_json::Value, key: &str) -> Option<String> {
    entry
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"[
        {
            "data": [
                {
                    "figi": "BBG000B9XRY4",
                    "name": "APPLE INC",
                    "ticker": "AAPL",
                    "exchCode": "US",
                    "securityType": "Common Stock"
                },
                {
                    "figi": "BBG000B9Y5X2",
                    "name": "APPLE INC",
                    "ticker": "AAPL",
                    "exchCode": "UW"
                }
            ]
        },
        {
            "warning": "No identifier found."
        }
    ]"#;

    #[test]
    fn parses_mapping_rows_and_skips_failed_jobs() {
        let rows = parse_openfigi_mapping(FIXTURE).expect("fixture parses");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            OpenFigiMapping {
                figi: "BBG000B9XRY4".to_string(),
                name: Some("APPLE INC".to_string()),
                ticker: Some("AAPL".to_string()),
                exch_code: Some("US".to_string()),
            }
        );
        assert_eq!(rows[1].exch_code.as_deref(), Some("UW"));
        assert!(rows[1].name.is_some());
    }

    #[test]
    fn skips_entries_without_a_figi() {
        let body = r#"[{"data":[{"ticker":"NOPE"},{"figi":"BBG000BWRKR8"}]}]"#;
        let rows = parse_openfigi_mapping(body).expect("parses");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].figi, "BBG000BWRKR8");
    }

    #[test]
    fn rejects_invalid_json_and_wrong_shape() {
        assert!(matches!(
            parse_openfigi_mapping("not json"),
            Err(OpenFigiParseError::InvalidJson(_))
        ));
        assert_eq!(
            parse_openfigi_mapping("{\"data\":[]}"),
            Err(OpenFigiParseError::UnexpectedShape)
        );
    }
}
