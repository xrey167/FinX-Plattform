#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-mask: Redact mode, Last4 with short values,
// multiple rules, unknown field skipped, idempotence.
// Authored offline. Verify with `cargo test --package tdw-mask`.

use std::collections::BTreeMap;

use tdw_hooks::TransactionMode;
use tdw_mask::{MaskMode, MaskRule, apply_masks, masking_hook};

fn row_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn redact_mode_replaces_value_with_three_stars() {
    let row = row_of(&[("ssn", "123-45-6789")]);
    let masked = apply_masks(
        &row,
        &[MaskRule {
            field: "ssn".to_string(),
            mode: MaskMode::Redact,
        }],
    );
    assert_eq!(masked.get("ssn"), Some(&"***".to_string()));
}

#[test]
fn last4_mode_keeps_final_four_characters() {
    let row = row_of(&[("account_id", "ACC123456")]);
    let masked = apply_masks(
        &row,
        &[MaskRule {
            field: "account_id".to_string(),
            mode: MaskMode::Last4,
        }],
    );
    assert_eq!(masked.get("account_id"), Some(&"***3456".to_string()));
}

#[test]
fn last4_with_value_shorter_than_four_preserves_all_chars() {
    let row = row_of(&[("pin", "ab")]);
    let masked = apply_masks(
        &row,
        &[MaskRule {
            field: "pin".to_string(),
            mode: MaskMode::Last4,
        }],
    );
    assert_eq!(masked.get("pin"), Some(&"***ab".to_string()));
}

#[test]
fn last4_with_empty_value_returns_three_stars_only() {
    let row = row_of(&[("pin", "")]);
    let masked = apply_masks(
        &row,
        &[MaskRule {
            field: "pin".to_string(),
            mode: MaskMode::Last4,
        }],
    );
    assert_eq!(masked.get("pin"), Some(&"***".to_string()));
}

#[test]
fn unknown_field_in_rule_is_a_noop() {
    let row = row_of(&[("symbol", "AAPL")]);
    let masked = apply_masks(
        &row,
        &[MaskRule {
            field: "missing".to_string(),
            mode: MaskMode::Redact,
        }],
    );
    assert_eq!(masked, row, "unknown rule field should not mutate the row");
}

#[test]
fn multiple_rules_apply_to_distinct_fields() {
    let row = row_of(&[
        ("account_id", "ACC123456"),
        ("ssn", "111-22-3333"),
        ("symbol", "AAPL"),
    ]);
    let masked = apply_masks(
        &row,
        &[
            MaskRule {
                field: "account_id".to_string(),
                mode: MaskMode::Last4,
            },
            MaskRule {
                field: "ssn".to_string(),
                mode: MaskMode::Redact,
            },
        ],
    );
    assert_eq!(masked.get("account_id"), Some(&"***3456".to_string()));
    assert_eq!(masked.get("ssn"), Some(&"***".to_string()));
    assert_eq!(masked.get("symbol"), Some(&"AAPL".to_string()));
}

#[test]
fn empty_rule_list_returns_clone_of_row() {
    let row = row_of(&[("a", "1"), ("b", "2")]);
    let masked = apply_masks(&row, &[]);
    assert_eq!(masked, row);
}

#[test]
fn applying_redact_twice_is_idempotent() {
    let row = row_of(&[("ssn", "123-45-6789")]);
    let rules = vec![MaskRule {
        field: "ssn".to_string(),
        mode: MaskMode::Redact,
    }];
    let once = apply_masks(&row, &rules);
    let twice = apply_masks(&once, &rules);
    assert_eq!(once, twice);
}

#[test]
fn masking_hook_is_in_transaction_with_order_five() {
    let hook = masking_hook();
    assert_eq!(hook.name, "mask.sync_filter");
    assert_eq!(hook.order, 5);
    assert_eq!(hook.transaction_mode, TransactionMode::InTransaction);
    assert!(hook.enabled);
}
