//! Shared query-parameter normalization for the `transform_query` stage.
//!
//! Every `Fetcher` receives raw caller arguments as a [`serde_json::Value`]
//! and must turn them into a typed query (the "T" — transform — step of the
//! TET pipeline). Before this module each provider re-parsed `start_date` /
//! `end_date` / `interval` / `limit` by hand, with subtly different error
//! text and validation. [`StandardParams`] centralizes that work so providers
//! call one helper and inherit consistent defaults, validation, and error
//! messages.
//!
//! The module is intentionally dependency-free: it parses and validates
//! calendar dates with a small in-crate civil-date routine rather than pulling
//! `chrono` / `time` into `tdw-core`, mirroring the deliberate choice already
//! made in the provider crates (see `tdw-provider-yahoo`'s `unix_to_iso_date`).
//! Future-date checks are therefore explicit: callers pass a reference "today"
//! to [`StandardParams::warnings`] so parsing stays pure and deterministic.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

/// Largest `limit` a caller may request. Keeps a single bad argument from
/// asking a provider for an unbounded result set.
pub const MAX_LIMIT: u32 = 100_000;

/// A validated calendar date in `YYYY-MM-DD` (ISO-8601) form.
///
/// Stores the parsed year/month/day so ordering comparisons are correct and
/// cheap, and round-trips through serde as the canonical zero-padded string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    /// Parse a `YYYY-MM-DD` string into a validated [`Date`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] when the string is not exactly
    /// `YYYY-MM-DD`, contains non-numeric components, or encodes a calendar
    /// date that does not exist (e.g. month `13` or `2025-02-30`).
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        let parts: Vec<&str> = trimmed.split('-').collect();
        if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
            return Err(Error::InvalidQuery(format!(
                "date must be in YYYY-MM-DD form, got {trimmed:?}"
            )));
        }
        let year = parts[0].parse::<i32>().map_err(|_| invalid_date(trimmed))?;
        let month = parts[1].parse::<u8>().map_err(|_| invalid_date(trimmed))?;
        let day = parts[2].parse::<u8>().map_err(|_| invalid_date(trimmed))?;
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return Err(invalid_date(trimmed));
        }
        Ok(Self { year, month, day })
    }

    /// The calendar year.
    #[must_use]
    pub const fn year(self) -> i32 {
        self.year
    }

    /// The calendar month, `1..=12`.
    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    /// The calendar day of month, `1..=31`.
    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl Serialize for Date {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Date {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("Date")
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Serialized form is an ISO `YYYY-MM-DD` date string.
        String::json_schema(generator)
    }
}

fn invalid_date(input: &str) -> Error {
    Error::InvalidQuery(format!("{input:?} is not a valid calendar date"))
}

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Bar / observation frequency, from one-minute intra-day up to one-month.
///
/// Parsing accepts the OpenBB-style tokens (`1m`, `5m`, `1h`, `1d`, `1W`,
/// `1M`) case-insensitively for the unit letter, except that month (`M`) and
/// minute (`m`) are disambiguated by case as they are everywhere in market
/// data conventions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Interval {
    /// One minute.
    #[serde(rename = "1m")]
    OneMinute,
    /// Five minutes.
    #[serde(rename = "5m")]
    FiveMinutes,
    /// Fifteen minutes.
    #[serde(rename = "15m")]
    FifteenMinutes,
    /// Thirty minutes.
    #[serde(rename = "30m")]
    ThirtyMinutes,
    /// One hour.
    #[serde(rename = "1h")]
    OneHour,
    /// One day.
    #[serde(rename = "1d")]
    #[default]
    OneDay,
    /// One week.
    #[serde(rename = "1W")]
    OneWeek,
    /// One month.
    #[serde(rename = "1M")]
    OneMonth,
}

impl Interval {
    /// Parse an interval token such as `"1m"`, `"1h"`, `"1d"`, `"1W"`, `"1M"`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] for any token outside the supported set.
    pub fn parse(input: &str) -> Result<Self> {
        let token = input.trim();
        let interval = match token {
            "1m" => Self::OneMinute,
            "5m" => Self::FiveMinutes,
            "15m" => Self::FifteenMinutes,
            "30m" => Self::ThirtyMinutes,
            "1h" => Self::OneHour,
            "1d" => Self::OneDay,
            // Accept the lowercase week form as a convenience; month stays
            // uppercase-only so it can never collide with minute.
            "1W" | "1w" => Self::OneWeek,
            "1M" => Self::OneMonth,
            other => {
                return Err(Error::InvalidQuery(format!(
                    "unsupported interval {other:?}; expected one of \
                     1m, 5m, 15m, 30m, 1h, 1d, 1W, 1M"
                )));
            }
        };
        Ok(interval)
    }

    /// The canonical token for this interval (the inverse of [`Interval::parse`]).
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::OneMinute => "1m",
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
            Self::OneHour => "1h",
            Self::OneDay => "1d",
            Self::OneWeek => "1W",
            Self::OneMonth => "1M",
        }
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_token())
    }
}

/// Reporting cadence for fundamentals-style endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    /// Annual reporting period.
    Annual,
    /// Quarterly reporting period.
    Quarter,
}

impl Period {
    /// Parse a period token. Accepts `annual` / `quarter` plus the common
    /// aliases `year`, `fy`, `q`, and `quarterly`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] for any other token.
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "annual" | "year" | "fy" => Ok(Self::Annual),
            "quarter" | "quarterly" | "q" => Ok(Self::Quarter),
            other => Err(Error::InvalidQuery(format!(
                "unsupported period {other:?}; expected annual or quarter"
            ))),
        }
    }

    /// The canonical token for this period.
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Annual => "annual",
            Self::Quarter => "quarter",
        }
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_token())
    }
}

/// Normalized, validated set of the date/interval/period/limit arguments that
/// recur across nearly every `Fetcher`.
///
/// Build one with [`StandardParams::from_value`] inside `transform_query`,
/// then read the typed fields. All cross-field validation (`end_date >=
/// start_date`, bounded `limit`) runs during construction; non-fatal advisory
/// checks (dates in the future) are surfaced separately by
/// [`StandardParams::warnings`] so the parse step stays clock-free and
/// deterministic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StandardParams {
    /// Inclusive lower date bound, if the caller supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<Date>,
    /// Inclusive upper date bound, if the caller supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<Date>,
    /// Bar / observation frequency (defaults to one day).
    #[serde(default)]
    pub interval: Interval,
    /// Reporting cadence for fundamentals-style endpoints, if relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    /// Caller-requested row cap, bounded by [`MAX_LIMIT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl StandardParams {
    /// Parse and validate the standard query parameters from raw caller
    /// arguments.
    ///
    /// Recognized keys: `start_date`, `end_date` (`YYYY-MM-DD` strings),
    /// `interval`, `period` (tokens), and `limit` (a non-negative integer).
    /// Unknown keys are ignored so a provider can mix these with its own
    /// arguments (e.g. `symbol`) in the same `Value`. Every field is optional;
    /// an empty object yields [`StandardParams::default`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] when a value has the wrong JSON type,
    /// a date / interval / period token fails to parse, `limit` is negative or
    /// exceeds [`MAX_LIMIT`], or `end_date` precedes `start_date`.
    pub fn from_value(params: &Value) -> Result<Self> {
        let start_date = parse_date_field(params, "start_date")?;
        let end_date = parse_date_field(params, "end_date")?;
        let interval = match params.get("interval") {
            None | Some(Value::Null) => Interval::default(),
            Some(Value::String(token)) => Interval::parse(token)?,
            Some(_) => return Err(wrong_type("interval", "a string")),
        };
        let period = match params.get("period") {
            None | Some(Value::Null) => None,
            Some(Value::String(token)) => Some(Period::parse(token)?),
            Some(_) => return Err(wrong_type("period", "a string")),
        };
        let limit = parse_limit_field(params)?;

        if let (Some(start), Some(end)) = (start_date, end_date)
            && end < start
        {
            return Err(Error::InvalidQuery(format!(
                "end_date {end} must be on or after start_date {start}"
            )));
        }

        Ok(Self {
            start_date,
            end_date,
            interval,
            period,
            limit,
        })
    }

    /// Advisory, non-fatal checks relative to a caller-supplied `today`.
    ///
    /// Returns human-readable warnings (currently: a `start_date` or
    /// `end_date` that is in the future). The list is empty when nothing is
    /// noteworthy. Kept separate from [`StandardParams::from_value`] so the
    /// parse path never needs a clock.
    #[must_use]
    pub fn warnings(&self, today: Date) -> Vec<String> {
        let mut warnings = Vec::new();
        if let Some(start) = self.start_date
            && start > today
        {
            warnings.push(format!(
                "start_date {start} is in the future (today is {today})"
            ));
        }
        if let Some(end) = self.end_date
            && end > today
        {
            warnings.push(format!(
                "end_date {end} is in the future (today is {today})"
            ));
        }
        warnings
    }
}

fn parse_date_field(params: &Value, key: &str) -> Result<Option<Date>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => Date::parse(raw).map(Some),
        Some(_) => Err(wrong_type(key, "a YYYY-MM-DD string")),
    }
}

fn parse_limit_field(params: &Value) -> Result<Option<u32>> {
    let Some(value) = params.get("limit") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw = value
        .as_i64()
        .ok_or_else(|| wrong_type("limit", "a non-negative integer"))?;
    if raw < 0 {
        return Err(Error::InvalidQuery(format!(
            "limit must be non-negative, got {raw}"
        )));
    }
    let limit = u32::try_from(raw).ok().filter(|value| *value <= MAX_LIMIT);
    match limit {
        Some(limit) => Ok(Some(limit)),
        None => Err(Error::InvalidQuery(format!(
            "limit {raw} exceeds the maximum of {MAX_LIMIT}"
        ))),
    }
}

fn wrong_type(key: &str, expected: &str) -> Error {
    Error::InvalidQuery(format!("{key} must be {expected}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_valid_dates_including_leap_day() {
        let date = Date::parse("2024-02-29").unwrap_or_else(|error| panic!("leap day: {error}"));
        assert_eq!((date.year(), date.month(), date.day()), (2024, 2, 29));
        assert_eq!(date.to_string(), "2024-02-29");
        // Trimming is applied before parsing.
        assert_eq!(
            Date::parse(" 2026-01-02 ").unwrap_or_else(|error| panic!("trim: {error}")),
            Date::parse("2026-01-02").unwrap_or_else(|error| panic!("plain: {error}"))
        );
    }

    #[test]
    fn rejects_malformed_or_impossible_dates() {
        assert!(Date::parse("2026-1-2").is_err()); // not zero-padded
        assert!(Date::parse("2026/01/02").is_err());
        assert!(Date::parse("2026-13-01").is_err());
        assert!(Date::parse("2026-02-30").is_err());
        assert!(Date::parse("2025-02-29").is_err()); // 2025 is not a leap year
        assert!(Date::parse("not-a-date").is_err());
    }

    #[test]
    fn date_ordering_is_calendar_correct() {
        let earlier = Date::parse("2026-01-31").unwrap_or_else(|error| panic!("{error}"));
        let later = Date::parse("2026-02-01").unwrap_or_else(|error| panic!("{error}"));
        assert!(earlier < later);
    }

    #[test]
    fn date_round_trips_through_serde_as_string() {
        let date = Date::parse("2026-06-07").unwrap_or_else(|error| panic!("{error}"));
        let encoded = serde_json::to_value(date).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(encoded, json!("2026-06-07"));
        let decoded: Date =
            serde_json::from_value(encoded).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(decoded, date);
    }

    #[test]
    fn interval_parses_and_renders_canonical_tokens() {
        for token in ["1m", "5m", "15m", "30m", "1h", "1d", "1W", "1M"] {
            let interval =
                Interval::parse(token).unwrap_or_else(|error| panic!("{token}: {error}"));
            assert_eq!(interval.as_token(), token);
            assert_eq!(interval.to_string(), token);
        }
        // Lowercase week is accepted; month is uppercase-only.
        assert_eq!(
            Interval::parse("1w").unwrap_or_else(|error| panic!("{error}")),
            Interval::OneWeek
        );
        assert!(Interval::parse("1y").is_err());
        assert!(Interval::parse("").is_err());
        assert_eq!(Interval::default(), Interval::OneDay);
    }

    #[test]
    fn period_parses_tokens_and_aliases() {
        for token in ["annual", "year", "fy", "Annual"] {
            assert_eq!(
                Period::parse(token).unwrap_or_else(|error| panic!("{token}: {error}")),
                Period::Annual
            );
        }
        for token in ["quarter", "quarterly", "q", "QUARTER"] {
            assert_eq!(
                Period::parse(token).unwrap_or_else(|error| panic!("{token}: {error}")),
                Period::Quarter
            );
        }
        assert!(Period::parse("monthly").is_err());
    }

    #[test]
    fn from_value_defaults_when_empty() {
        let params =
            StandardParams::from_value(&json!({})).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(params, StandardParams::default());
        assert_eq!(params.interval, Interval::OneDay);
        assert!(params.start_date.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn from_value_parses_full_payload() {
        let params = StandardParams::from_value(&json!({
            "symbol": "AAPL",
            "start_date": "2026-01-01",
            "end_date": "2026-03-31",
            "interval": "1h",
            "period": "quarter",
            "limit": 500
        }))
        .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(
            params.start_date.map(|d| d.to_string()).as_deref(),
            Some("2026-01-01")
        );
        assert_eq!(
            params.end_date.map(|d| d.to_string()).as_deref(),
            Some("2026-03-31")
        );
        assert_eq!(params.interval, Interval::OneHour);
        assert_eq!(params.period, Some(Period::Quarter));
        assert_eq!(params.limit, Some(500));
    }

    #[test]
    fn from_value_rejects_end_before_start() {
        let error = StandardParams::from_value(&json!({
            "start_date": "2026-03-01",
            "end_date": "2026-02-01"
        }))
        .expect_err("end before start must fail");
        let message = error.to_string();
        assert!(message.contains("on or after"), "got: {message}");
    }

    #[test]
    fn from_value_rejects_wrong_types_and_bad_limits() {
        assert!(StandardParams::from_value(&json!({ "start_date": 20260101 })).is_err());
        assert!(StandardParams::from_value(&json!({ "interval": 60 })).is_err());
        assert!(StandardParams::from_value(&json!({ "limit": -5 })).is_err());
        assert!(StandardParams::from_value(&json!({ "limit": "many" })).is_err());
        let over = StandardParams::from_value(&json!({ "limit": i64::from(MAX_LIMIT) + 1 }))
            .expect_err("over-limit must fail");
        assert!(over.to_string().contains("maximum"), "got: {over}");
    }

    #[test]
    fn warnings_flag_future_dates_only() {
        let today = Date::parse("2026-06-07").unwrap_or_else(|error| panic!("{error}"));
        let params = StandardParams::from_value(&json!({
            "start_date": "2026-01-01",
            "end_date": "2030-01-01"
        }))
        .unwrap_or_else(|error| panic!("{error}"));
        let warnings = params.warnings(today);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("end_date 2030-01-01"));

        let no_future = StandardParams::from_value(&json!({ "end_date": "2026-06-07" }))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(no_future.warnings(today).is_empty());
    }
}
