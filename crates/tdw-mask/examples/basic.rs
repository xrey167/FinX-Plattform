//! Offline `tdw-mask` example: input row -> masked output, plus the fail-closed
//! fallback when a rule is invalid.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-mask --example tdw_mask_basic
//! ```

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use tdw_mask::{MaskMode, MaskRule, apply_masks, masking_hook, try_apply_masks};

fn sample_row() -> BTreeMap<String, String> {
    let mut row = BTreeMap::new();
    row.insert("account_id".to_string(), "ACC123456".to_string());
    row.insert("symbol".to_string(), "AAPL".to_string());
    row
}

fn main() {
    let row = sample_row();

    // Last4 keeps the trailing four chars; unnamed fields pass through.
    let masked = apply_masks(
        &row,
        &[MaskRule {
            field: "account_id".to_string(),
            mode: MaskMode::Last4,
        }],
    );
    assert_eq!(masked.get("account_id"), Some(&"***3456".to_string()));
    assert_eq!(masked.get("symbol"), Some(&"AAPL".to_string()));
    println!("masked account_id -> {}", masked["account_id"]);

    // Fail-closed: an invalid field name makes `apply_masks` redact EVERYTHING.
    let invalid = [MaskRule {
        field: "account-id".to_string(), // '-' is not allowed in a field name
        mode: MaskMode::Last4,
    }];
    let safe = apply_masks(&row, &invalid);
    assert_eq!(safe.get("account_id"), Some(&"***".to_string()));
    assert_eq!(safe.get("symbol"), Some(&"***".to_string()));
    println!("invalid rule -> all fields redacted (fail-closed)");

    // The fallible form surfaces the typed error instead.
    assert!(try_apply_masks(&row, &invalid).is_err());

    // The crate exposes its masking step as a hook spec.
    assert_eq!(masking_hook().name, "mask.sync_filter");
    println!("masking hook: {}", masking_hook().name);
}
