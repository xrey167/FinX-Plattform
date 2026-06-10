//! The orthogonal Origin (tier × source) classification scheme.
//!
//! `Origin` classifies an entity *kind* itself — how built-in/specialized it is and
//! whether it bridges outside the platform. It is NOT lineage: where a particular piece
//! of *data* came from is recorded by a separate `provenance` field.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Origin tier — how built-in or specialized an entity kind is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Tier {
    /// Platform primitive.
    System,
    /// Finance/trading-domain specific.
    Domain,
    /// User-authored at runtime.
    Custom,
}

/// Origin source — whether the kind is defined inside the platform or bridges outside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Source {
    /// Defined inside the platform/user.
    Internal,
    /// Wraps or mirrors a third-party standard or service.
    External,
}

/// Orthogonal entity classification.
///
/// This is NOT lineage: where a particular piece of *data* came from is recorded by a
/// separate `provenance` field. `Origin` classifies the kind itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Origin {
    /// How built-in/specialized the kind is.
    pub tier: Tier,
    /// Whether the kind bridges outside the platform.
    pub source: Source,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_round_trips_tier_and_source() {
        let origin = Origin {
            tier: Tier::Domain,
            source: Source::External,
        };
        let encoded = serde_json::to_value(origin).expect("origin should serialize");
        assert_eq!(encoded["tier"], "Domain");
        assert_eq!(encoded["source"], "External");
        let decoded: Origin = serde_json::from_value(encoded).expect("origin should deserialize");
        assert_eq!(decoded, origin);
    }
}
