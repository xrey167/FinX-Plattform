//! Public-API coverage for `query_params`: the typed token round-trips,
//! schema emission, future-date advisories, and the `null`/wrong-type edges of
//! `StandardParams::from_value` that the in-module unit tests do not exercise.

use schemars::schema_for;
use serde_json::json;

use tdw_core::query_params::{Date, Interval, Period};
use tdw_core::{Error, StandardParams};

#[test]
fn period_round_trips_through_token_and_display() {
    for (period, token) in [(Period::Annual, "annual"), (Period::Quarter, "quarter")] {
        assert_eq!(period.as_token(), token);
        assert_eq!(period.to_string(), token);
        assert_eq!(
            Period::parse(token).expect("period token should parse"),
            period
        );
    }
}

#[test]
fn interval_round_trips_through_token_and_display() {
    for token in ["1m", "5m", "15m", "30m", "1h", "1d", "1W", "1M"] {
        let parsed = Interval::parse(token).expect("interval token should parse");
        assert_eq!(parsed.as_token(), token);
        assert_eq!(parsed.to_string(), token);
    }
}

#[test]
fn date_json_schema_is_a_string() {
    // Exercises the hand-written `JsonSchema` impl for `Date`.
    let schema = schema_for!(Date);
    let value = serde_json::to_value(&schema).expect("date schema should serialize");
    assert_eq!(value.get("type").and_then(|t| t.as_str()), Some("string"));
}

#[test]
fn standard_params_warns_on_future_start_and_end_dates() {
    let params = StandardParams::from_value(&json!({
        "start_date": "2999-01-02",
        "end_date": "2999-03-04",
    }))
    .expect("future dates should parse as standard params");
    let today = Date::parse("2026-06-08").expect("fixed today date should parse");
    let warnings = params.warnings(today);
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].contains("start_date 2999-01-02 is in the future"));
    assert!(warnings[1].contains("end_date 2999-03-04 is in the future"));
}

#[test]
fn standard_params_has_no_warnings_for_past_dates() {
    let params = StandardParams::from_value(&json!({
        "start_date": "2000-01-01",
        "end_date": "2000-12-31",
    }))
    .expect("past dates should parse as standard params");
    let today = Date::parse("2026-06-08").expect("fixed today date should parse");
    assert!(params.warnings(today).is_empty());
}

#[test]
fn explicit_null_limit_parses_as_absent() {
    let params =
        StandardParams::from_value(&json!({ "limit": null })).expect("null limit should parse");
    assert_eq!(params.limit, None);
}

#[test]
fn non_string_period_is_a_type_error() {
    let err = StandardParams::from_value(&json!({ "period": 4 }))
        .expect_err("numeric period should be rejected");
    match err {
        Error::InvalidQuery(message) => assert!(message.contains("period")),
        other => panic!("expected InvalidQuery, got {other:?}"),
    }
}

#[test]
fn non_string_interval_is_a_type_error() {
    let err = StandardParams::from_value(&json!({ "interval": true }))
        .expect_err("boolean interval should be rejected");
    match err {
        Error::InvalidQuery(message) => assert!(message.contains("interval")),
        other => panic!("expected InvalidQuery, got {other:?}"),
    }
}
