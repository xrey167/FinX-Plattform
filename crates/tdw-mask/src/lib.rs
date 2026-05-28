#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tdw_hooks::{HookSpec, TransactionMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskError {
    InvalidFieldName,
}

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

#[must_use]
pub fn apply_masks(row: &BTreeMap<String, String>, rules: &[MaskRule]) -> BTreeMap<String, String> {
    try_apply_masks(row, rules).unwrap_or_else(|_| redact_all(row))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn try_apply_masks(
    row: &BTreeMap<String, String>,
    rules: &[MaskRule],
) -> Result<BTreeMap<String, String>, MaskError> {
    validate_rules(rules)?;
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
    Ok(masked)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_rules(rules: &[MaskRule]) -> Result<(), MaskError> {
    if rules.iter().any(|rule| !is_field_name(&rule.field)) {
        return Err(MaskError::InvalidFieldName);
    }
    Ok(())
}

#[must_use]
pub fn masking_hook() -> HookSpec {
    HookSpec::new("mask.sync_filter", 5, TransactionMode::InTransaction)
}

fn is_field_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(is_field_character))
}

const fn is_field_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn redact_all(row: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    row.keys()
        .map(|field| (field.clone(), "***".to_string()))
        .collect()
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

    #[test]
    fn compatibility_wrapper_fails_closed_on_invalid_rules() {
        let mut row = BTreeMap::new();
        row.insert("account_id".to_string(), "ACC123456".to_string());

        let masked = apply_masks(
            &row,
            &[MaskRule {
                field: "account-id".to_string(),
                mode: MaskMode::Last4,
            }],
        );

        assert_eq!(masked.get("account_id"), Some(&"***".to_string()));
    }

    #[test]
    fn rejects_unsafe_mask_field_names() {
        let row = BTreeMap::new();
        let rejected = try_apply_masks(
            &row,
            &[MaskRule {
                field: "account_id;drop".to_string(),
                mode: MaskMode::Redact,
            }],
        );

        assert_eq!(rejected, Err(MaskError::InvalidFieldName));
    }
}
