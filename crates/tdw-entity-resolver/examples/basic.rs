//! Offline `tdw-entity-resolver` example: resolve an instrument two ways — by
//! ticker against knowledge-graph entities, and by a standardized identifier
//! against an in-memory crosswalk.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-entity-resolver --example basic
//! ```

use tdw_entity_resolver::{IdentifierRecord, resolve_by_identifier, resolve_symbol};
use tdw_kg::{Entity, EntityKind};

fn main() {
    // Record 1: a knowledge-graph instrument with a ticker alias.
    let entities = vec![Entity {
        entity_id: "instrument:AAPL".to_string(),
        kind: EntityKind::Instrument,
        label: "Apple".to_string(),
        aliases: vec!["AAPL".to_string()],
    }];

    // Resolve by ticker (case-insensitive).
    let by_symbol = resolve_symbol("aapl", &entities);
    println!(
        "symbol 'aapl' -> {} (score {})",
        by_symbol[0].entity_id, by_symbol[0].score
    );

    // Record 2: a standardized-identifier crosswalk row.
    let records = vec![IdentifierRecord {
        scheme: "FIGI".to_string(),
        value: "BBG000B9XRY4".to_string(),
        instrument_id: "INST-AAPL-XNAS".to_string(),
    }];

    // Resolve by FIGI (case-insensitive scheme + value).
    let by_figi = resolve_by_identifier("figi", "BBG000B9XRY4", &records);
    println!(
        "FIGI 'BBG000B9XRY4' -> {} ({})",
        by_figi[0].entity_id, by_figi[0].reason
    );

    // A non-existent identifier resolves to nothing.
    println!(
        "unknown FIGI matches: {}",
        resolve_by_identifier("FIGI", "NOPE00000000", &records).len()
    );
}
