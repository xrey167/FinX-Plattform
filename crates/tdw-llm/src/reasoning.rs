//! Typed per-message reasoning-level directive resolution.
//!
//! This module is purely additive: it does not touch the [`crate::LanguageModel`]
//! trait or any client crate. It provides a small, dependency-free vocabulary for
//! expressing how much "thinking" a request should request from a model, a
//! precedence resolver for combining inline / session / agent / default sources,
//! and a mapping from a resolved level to provider-advisory parameters.
//!
//! All functions are pure (no I/O, no async, no provider calls). The values
//! returned by [`to_params`] are *advisory* hints: `thinking_budget_tokens`
//! mirrors the Anthropic extended-thinking budget and `reasoning_effort` mirrors
//! the `OpenAI` reasoning-effort knob. Wiring these into the concrete client crates
//! is intentionally deferred to a follow-up; nothing here performs a network call.

use serde::{Deserialize, Serialize};

/// How much reasoning effort a request should ask the model to spend.
///
/// The default is [`ReasoningLevel::Medium`], matching a balanced effort that is
/// reasonable for most interactive requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningLevel {
    /// No extended reasoning requested.
    Off,
    /// Minimal reasoning effort.
    Low,
    /// Balanced reasoning effort (the default).
    #[default]
    Medium,
    /// Elevated reasoning effort.
    High,
    /// Maximum reasoning effort.
    Max,
}

impl ReasoningLevel {
    /// Parses a single level token (case-insensitive). Returns `None` for any
    /// token that does not name a known level.
    #[must_use]
    pub fn parse_token(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

/// Configured reasoning-level sources, ordered by increasing generality.
///
/// `session` and `agent` are optional overrides; `default` is the floor that is
/// always present. Inline directives parsed from a message take precedence over
/// every field here (see [`resolve`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningConfig {
    /// Per-session override, if any.
    pub session: Option<ReasoningLevel>,
    /// Per-agent override, if any.
    pub agent: Option<ReasoningLevel>,
    /// The always-present fallback level.
    pub default: ReasoningLevel,
}

/// Provider-advisory parameters derived from a [`ReasoningLevel`].
///
/// These are plain data. `thinking_budget_tokens` mirrors the Anthropic
/// extended-thinking budget and `reasoning_effort` mirrors the `OpenAI`
/// reasoning-effort string. Both are advisory and are not consumed by any client
/// crate in this unit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReasoningParams {
    /// Sampling temperature; always within `[0.0, 2.0]`.
    pub temperature: f32,
    /// Advisory Anthropic extended-thinking budget in tokens, if applicable.
    pub thinking_budget_tokens: Option<u32>,
    /// Advisory `OpenAI` reasoning-effort hint, if applicable.
    pub reasoning_effort: Option<&'static str>,
}

/// Strips a leading `/think:<level>` directive from `message`, returning the
/// parsed level (if recognized) and the remaining message text.
///
/// The parse is tolerant:
/// - Leading whitespace before the directive is ignored.
/// - An unknown `<level>` yields `(None, original_message)` with the message
///   returned unchanged (directive *not* stripped).
/// - A message with no directive yields `(None, original_message)`.
///
/// When a recognized directive is present, the returned message is the remainder
/// with one run of separating whitespace trimmed from its front.
#[must_use]
pub fn parse_directive(message: &str) -> (Option<ReasoningLevel>, &str) {
    let trimmed = message.trim_start();
    let Some(rest) = trimmed.strip_prefix("/think:") else {
        return (None, message);
    };

    // The level token runs up to the first whitespace; the remainder is the body.
    let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (token, remainder) = rest.split_at(token_end);

    // Unknown level: tolerate by passing the original message through unchanged.
    ReasoningLevel::parse_token(token).map_or((None, message), |level| {
        (Some(level), remainder.trim_start())
    })
}

/// Resolves the effective reasoning level using precedence
/// `inline > session > agent > default`.
#[must_use]
pub fn resolve(inline: Option<ReasoningLevel>, cfg: &ReasoningConfig) -> ReasoningLevel {
    inline.or(cfg.session).or(cfg.agent).unwrap_or(cfg.default)
}

/// Maps a resolved [`ReasoningLevel`] to advisory provider parameters.
///
/// The returned temperature is always within `[0.0, 2.0]` (see
/// [`clamp_temperature`]). For [`ReasoningLevel::Off`], both
/// `thinking_budget_tokens` and `reasoning_effort` are `None`.
#[must_use]
pub const fn to_params(level: ReasoningLevel) -> ReasoningParams {
    let (temperature, thinking_budget_tokens, reasoning_effort) = match level {
        ReasoningLevel::Off => (0.0, None, None),
        ReasoningLevel::Low => (0.5, Some(1_024), Some("low")),
        ReasoningLevel::Medium => (0.7, Some(4_096), Some("medium")),
        ReasoningLevel::High => (1.0, Some(16_384), Some("high")),
        ReasoningLevel::Max => (1.0, Some(32_768), Some("high")),
    };
    ReasoningParams {
        temperature: clamp_temperature(temperature),
        thinking_budget_tokens,
        reasoning_effort,
    }
}

/// Clamps a temperature into the supported `[0.0, 2.0]` range.
///
/// `NaN` is mapped to `0.0` (the conservative floor) so callers never propagate a
/// non-comparable temperature to a provider.
#[must_use]
pub const fn clamp_temperature(temperature: f32) -> f32 {
    if temperature.is_nan() {
        return 0.0;
    }
    temperature.clamp(0.0, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_level_is_medium() {
        assert_eq!(ReasoningLevel::default(), ReasoningLevel::Medium);
    }

    #[test]
    fn default_config_is_default_medium_no_overrides() {
        let cfg = ReasoningConfig::default();
        assert_eq!(cfg.session, None);
        assert_eq!(cfg.agent, None);
        assert_eq!(cfg.default, ReasoningLevel::Medium);
    }

    #[test]
    fn parse_directive_each_level() {
        assert_eq!(
            parse_directive("/think:off body"),
            (Some(ReasoningLevel::Off), "body")
        );
        assert_eq!(
            parse_directive("/think:low body"),
            (Some(ReasoningLevel::Low), "body")
        );
        assert_eq!(
            parse_directive("/think:medium body"),
            (Some(ReasoningLevel::Medium), "body")
        );
        assert_eq!(
            parse_directive("/think:high body"),
            (Some(ReasoningLevel::High), "body")
        );
        assert_eq!(
            parse_directive("/think:max body"),
            (Some(ReasoningLevel::Max), "body")
        );
    }

    #[test]
    fn parse_directive_is_case_insensitive() {
        assert_eq!(
            parse_directive("/think:HIGH go"),
            (Some(ReasoningLevel::High), "go")
        );
    }

    #[test]
    fn parse_directive_unknown_level_is_tolerated() {
        // Unknown level => None and the original message returned unchanged.
        let original = "/think:turbo do the thing";
        assert_eq!(parse_directive(original), (None, original));
    }

    #[test]
    fn parse_directive_no_directive_passthrough() {
        let original = "just a normal message";
        assert_eq!(parse_directive(original), (None, original));
    }

    #[test]
    fn parse_directive_leading_whitespace() {
        assert_eq!(
            parse_directive("   /think:high   payload here"),
            (Some(ReasoningLevel::High), "payload here")
        );
    }

    #[test]
    fn parse_directive_token_only_no_body() {
        assert_eq!(
            parse_directive("/think:max"),
            (Some(ReasoningLevel::Max), "")
        );
    }

    #[test]
    fn resolve_inline_wins_over_all() {
        let cfg = ReasoningConfig {
            session: Some(ReasoningLevel::Low),
            agent: Some(ReasoningLevel::High),
            default: ReasoningLevel::Off,
        };
        assert_eq!(
            resolve(Some(ReasoningLevel::Max), &cfg),
            ReasoningLevel::Max
        );
    }

    #[test]
    fn resolve_session_wins_over_agent_and_default() {
        let cfg = ReasoningConfig {
            session: Some(ReasoningLevel::Low),
            agent: Some(ReasoningLevel::High),
            default: ReasoningLevel::Off,
        };
        assert_eq!(resolve(None, &cfg), ReasoningLevel::Low);
    }

    #[test]
    fn resolve_agent_wins_over_default() {
        let cfg = ReasoningConfig {
            session: None,
            agent: Some(ReasoningLevel::High),
            default: ReasoningLevel::Off,
        };
        assert_eq!(resolve(None, &cfg), ReasoningLevel::High);
    }

    #[test]
    fn resolve_default_when_nothing_set() {
        let cfg = ReasoningConfig {
            session: None,
            agent: None,
            default: ReasoningLevel::Medium,
        };
        assert_eq!(resolve(None, &cfg), ReasoningLevel::Medium);
    }

    #[test]
    fn resolve_inline_over_session_only() {
        let cfg = ReasoningConfig {
            session: Some(ReasoningLevel::Low),
            agent: None,
            default: ReasoningLevel::Off,
        };
        assert_eq!(
            resolve(Some(ReasoningLevel::High), &cfg),
            ReasoningLevel::High
        );
    }

    #[test]
    fn resolve_inline_over_agent_only() {
        let cfg = ReasoningConfig {
            session: None,
            agent: Some(ReasoningLevel::Low),
            default: ReasoningLevel::Off,
        };
        assert_eq!(
            resolve(Some(ReasoningLevel::High), &cfg),
            ReasoningLevel::High
        );
    }

    #[test]
    fn resolve_inline_over_default_only() {
        let cfg = ReasoningConfig {
            session: None,
            agent: None,
            default: ReasoningLevel::Off,
        };
        assert_eq!(
            resolve(Some(ReasoningLevel::High), &cfg),
            ReasoningLevel::High
        );
    }

    #[test]
    fn clamp_temperature_below_zero() {
        assert!((clamp_temperature(-1.0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn clamp_temperature_above_two() {
        assert!((clamp_temperature(5.0) - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn clamp_temperature_in_range() {
        assert!((clamp_temperature(1.3) - 1.3).abs() < f32::EPSILON);
    }

    #[test]
    fn clamp_temperature_boundaries() {
        assert!((clamp_temperature(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((clamp_temperature(2.0) - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn clamp_temperature_nan_maps_to_zero() {
        assert!((clamp_temperature(f32::NAN) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn to_params_off_has_no_advisory_hints() {
        let params = to_params(ReasoningLevel::Off);
        assert!((params.temperature - 0.0).abs() < f32::EPSILON);
        assert_eq!(params.thinking_budget_tokens, None);
        assert_eq!(params.reasoning_effort, None);
    }

    #[test]
    fn to_params_low() {
        let params = to_params(ReasoningLevel::Low);
        assert_eq!(params.thinking_budget_tokens, Some(1_024));
        assert_eq!(params.reasoning_effort, Some("low"));
        assert!(params.temperature >= 0.0 && params.temperature <= 2.0);
    }

    #[test]
    fn to_params_medium() {
        let params = to_params(ReasoningLevel::Medium);
        assert_eq!(params.thinking_budget_tokens, Some(4_096));
        assert_eq!(params.reasoning_effort, Some("medium"));
        assert!(params.temperature >= 0.0 && params.temperature <= 2.0);
    }

    #[test]
    fn to_params_high() {
        let params = to_params(ReasoningLevel::High);
        assert_eq!(params.thinking_budget_tokens, Some(16_384));
        assert_eq!(params.reasoning_effort, Some("high"));
        assert!(params.temperature >= 0.0 && params.temperature <= 2.0);
    }

    #[test]
    fn to_params_max() {
        let params = to_params(ReasoningLevel::Max);
        assert_eq!(params.thinking_budget_tokens, Some(32_768));
        assert_eq!(params.reasoning_effort, Some("high"));
        assert!(params.temperature >= 0.0 && params.temperature <= 2.0);
    }

    #[test]
    fn to_params_all_variants_clamped_temperature() {
        for level in [
            ReasoningLevel::Off,
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::Max,
        ] {
            let params = to_params(level);
            assert!(
                params.temperature >= 0.0 && params.temperature <= 2.0,
                "temperature out of range for {level:?}"
            );
        }
    }

    #[test]
    fn serde_round_trip_level() {
        let json = serde_json::to_string(&ReasoningLevel::High)
            .unwrap_or_else(|error| panic!("serialize: {error}"));
        assert_eq!(json, "\"high\"");
        let parsed: ReasoningLevel =
            serde_json::from_str(&json).unwrap_or_else(|error| panic!("deserialize: {error}"));
        assert_eq!(parsed, ReasoningLevel::High);
    }
}
