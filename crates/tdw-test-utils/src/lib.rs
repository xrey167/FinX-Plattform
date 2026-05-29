#![forbid(unsafe_code)]

pub mod smoke;

use tdw_domain::{EquityHistoricalData, Instrument, ResearchNote};

pub mod fixtures {
    use super::{EquityHistoricalData, Instrument, ResearchNote};

    #[must_use]
    pub fn ohlcv(symbol: &str) -> Vec<EquityHistoricalData> {
        vec![
            EquityHistoricalData {
                symbol: symbol.to_string(),
                date: "2026-05-20".to_string(),
                open: 100.0,
                high: 102.0,
                low: 99.0,
                close: 101.0,
                volume: 10_000,
            },
            EquityHistoricalData {
                symbol: symbol.to_string(),
                date: "2026-05-21".to_string(),
                open: 101.0,
                high: 103.0,
                low: 100.0,
                close: 102.0,
                volume: 12_000,
            },
        ]
    }

    #[must_use]
    pub fn instrument(symbol: &str) -> Instrument {
        Instrument {
            symbol: symbol.to_string(),
            name: format!("{symbol} common stock"),
            venue: "XNAS".to_string(),
        }
    }

    #[must_use]
    pub fn research_note(id: &str) -> ResearchNote {
        ResearchNote {
            id: id.to_string(),
            title: "Synthetic macro note".to_string(),
            body: "Fixture note for deterministic tests.".to_string(),
            tags: vec!["fixture".to_string(), "research".to_string()],
        }
    }
}

pub mod containers {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ContainerSpec {
        pub name: &'static str,
        pub image: &'static str,
        pub default_port: u16,
    }

    #[must_use]
    pub const fn postgres() -> ContainerSpec {
        ContainerSpec {
            name: "postgres",
            image: "postgres:17-alpine",
            default_port: 5432,
        }
    }

    #[must_use]
    pub const fn clickhouse() -> ContainerSpec {
        ContainerSpec {
            name: "clickhouse",
            image: "clickhouse/clickhouse-server:25.5",
            default_port: 8123,
        }
    }

    #[must_use]
    pub const fn qdrant() -> ContainerSpec {
        ContainerSpec {
            name: "qdrant",
            image: "qdrant/qdrant:latest",
            default_port: 6333,
        }
    }

    #[must_use]
    pub const fn meilisearch() -> ContainerSpec {
        ContainerSpec {
            name: "meilisearch",
            image: "getmeili/meilisearch:latest",
            default_port: 7700,
        }
    }

    #[must_use]
    pub const fn minio() -> ContainerSpec {
        ContainerSpec {
            name: "minio",
            image: "minio/minio:latest",
            default_port: 9000,
        }
    }

    #[must_use]
    pub const fn redis() -> ContainerSpec {
        ContainerSpec {
            name: "redis",
            image: "redis:7-alpine",
            default_port: 6379,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ohlcv_fixture_is_deterministic() {
        assert_eq!(fixtures::ohlcv("AAPL"), fixtures::ohlcv("AAPL"));
    }

    #[test]
    fn container_specs_cover_minimal_profile() {
        assert_eq!(containers::postgres().default_port, 5432);
        assert_eq!(containers::clickhouse().default_port, 8123);
    }
}
