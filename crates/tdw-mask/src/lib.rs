#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tdw_hooks::{HookSpec, TransactionMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskMode {
    Redact,
    Last4,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskRule {
    pub field: String,
    pub mode: MaskMode,
}

pub fn apply_masks(row: &BTreeMap<String, String>, rules: &[MaskRule]) -> BTreeMap<String, String> {
    let mut masked = row.clone();
    for rule in rules {
        if let Some(value) = masked.get_mut(&rule.field) {
            *value = match rule.mode {
                MaskMode::Redact => "***".to_string(),
                MaskMode::Last4 => {
                    let keep = value.chars().rev().take(4).collect::<Vec<_>>();
                    let suffix = keep.into_iter().rev().collect::<String>();
                    format!("***{suffix}")
                }
            };
        }
    }
    masked
}

pub fn masking_hook() -> HookSpec {
    HookSpec::new("mask.sync_filter", 5, TransactionMode::InTransaction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_fields_and_exposes_sync_filter_hook() {
        let mut row = BTreeMap::new();
        row.insert("account_id".to_string(), "ACC123456".to_string());
        row.insert("symbol".to_string(), "AAPL".to_string());
        let masked = apply_masks(
            &row,
            &[MaskRule {
                field: "account_id".to_string(),
                mode: MaskMode::Last4,
            }],
        );

        assert_eq!(masked.get("account_id"), Some(&"***3456".to_string()));
        assert_eq!(masking_hook().name, "mask.sync_filter");
    }
}
