//! Configuration for the K-R5 rule-induction pipeline.
//!
//! [`InductionConfig`] mirrors the `[knowledge.induction]` TOML section.
//! The most important field is [`InductionConfig::enabled`], which defaults
//! to `false` — this is the most powerful new capability and the operator
//! must opt in explicitly.
//!
//! # Example TOML
//!
//! ```toml
//! [knowledge.induction]
//! enabled = true
//! cadence = "0 3 * * *"          # nightly at 03:00
//! min_promote_precision = 0.60
//! min_promote_recall    = 0.20
//! max_candidates_per_cycle = 20
//! retire_induced_on_disable = false
//! ```

use serde::{Deserialize, Serialize};

/// Configuration for the K-R5 pattern-→-rule induction pipeline.
///
/// # Important: enabled defaults to `false`
///
/// Rule induction is the most powerful new capability — it lets the system
/// write its own inference rules from mined patterns. The operator must
/// explicitly set `enabled = true` to activate. A loud NOTICE is emitted
/// at every cron tick when disabled so the operator knows the feature exists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InductionConfig {
    /// Whether the cron-driven induction pipeline is active.
    ///
    /// **Default: `false`.** Must be explicitly set to `true` by the
    /// operator. When `false`, a loud NOTICE is emitted on every tick.
    #[serde(default)]
    pub enabled: bool,

    /// 5-field cron expression for the induction cadence.
    ///
    /// Runs AFTER pattern mining (K-R4). Default: `"0 3 * * *"` (daily
    /// 03:00 UTC, one hour after the default mining cadence at 02:00).
    #[serde(default = "InductionConfig::default_cadence")]
    pub cadence: String,

    /// Minimum mean precision over replay splits for a candidate to be
    /// promoted. Default: `0.60` (matches `PromoteThreshold::default()`).
    #[serde(default = "InductionConfig::default_min_precision")]
    pub min_promote_precision: f64,

    /// Minimum mean recall over replay splits for a candidate to be
    /// promoted. Default: `0.20` (matches `PromoteThreshold::default()`).
    #[serde(default = "InductionConfig::default_min_recall")]
    pub min_promote_recall: f64,

    /// Maximum number of candidate rules processed per induction cycle.
    ///
    /// Hard cap — exceeding it is an [`InductionError::CandidateBudgetExceeded`],
    /// never silent truncation (B7 posture). Default: `20`.
    #[serde(default = "InductionConfig::default_max_candidates")]
    pub max_candidates_per_cycle: usize,

    /// When `true` AND `enabled` transitions from `true` to `false`, all
    /// previously inducted rules are removed from the inference engine on
    /// the next tick. Default: `false` (inducted rules persist until the
    /// next full reload even when induction is turned off).
    #[serde(default)]
    pub retire_induced_on_disable: bool,
}

impl InductionConfig {
    fn default_cadence() -> String {
        "0 3 * * *".to_string()
    }

    const fn default_min_precision() -> f64 {
        0.60
    }

    const fn default_min_recall() -> f64 {
        0.20
    }

    const fn default_max_candidates() -> usize {
        20
    }
}

impl Default for InductionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cadence: Self::default_cadence(),
            min_promote_precision: Self::default_min_precision(),
            min_promote_recall: Self::default_min_recall(),
            max_candidates_per_cycle: Self::default_max_candidates(),
            retire_induced_on_disable: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled() {
        let cfg = InductionConfig::default();
        assert!(!cfg.enabled, "induction must default to disabled");
    }

    #[test]
    fn default_thresholds_match_promote_gate_defaults() {
        let cfg = InductionConfig::default();
        // Must match PromoteThreshold::default() in tdw-eval-runner
        assert!((cfg.min_promote_precision - 0.60).abs() < f64::EPSILON);
        assert!((cfg.min_promote_recall - 0.20).abs() < f64::EPSILON);
    }

    #[test]
    fn deserializes_with_serde_defaults_from_empty_object() {
        let cfg: InductionConfig =
            serde_json::from_str("{}").expect("empty object must deserialize");
        assert!(!cfg.enabled);
        assert_eq!(cfg.cadence, "0 3 * * *");
        assert!((cfg.min_promote_precision - 0.60).abs() < f64::EPSILON);
        assert!((cfg.min_promote_recall - 0.20).abs() < f64::EPSILON);
        assert_eq!(cfg.max_candidates_per_cycle, 20);
        assert!(!cfg.retire_induced_on_disable);
    }

    #[test]
    fn round_trips_through_json() {
        let cfg = InductionConfig {
            enabled: true,
            cadence: "0 4 * * MON".to_string(),
            min_promote_precision: 0.75,
            min_promote_recall: 0.30,
            max_candidates_per_cycle: 10,
            retire_induced_on_disable: true,
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        let restored: InductionConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cfg, restored);
    }
}
