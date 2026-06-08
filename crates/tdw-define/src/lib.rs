#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_hooks::{HookSpec, TransactionMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefineError {
    InvalidEventName,
    InvalidTableName,
    InvalidHookName,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefineEvent {
    pub event_name: String,
    pub on_table: String,
    pub hook_name: String,
    pub transaction_mode: TransactionMode,
}

impl DefineEvent {
    #[must_use]
    pub fn compile_hook(&self) -> HookSpec {
        HookSpec::new(self.hook_name.clone(), 100, self.transaction_mode)
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_compile_hook(&self) -> Result<HookSpec, DefineError> {
        self.validate()?;
        Ok(self.compile_hook())
    }

    #[must_use]
    pub fn idempotency_key(&self) -> String {
        format!("{}:{}:{}", self.on_table, self.event_name, self.hook_name)
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn validate(&self) -> Result<(), DefineError> {
        if !is_action_name(&self.event_name) {
            return Err(DefineError::InvalidEventName);
        }
        if !is_table_name(&self.on_table) {
            return Err(DefineError::InvalidTableName);
        }
        if !is_action_name(&self.hook_name) {
            return Err(DefineError::InvalidHookName);
        }
        Ok(())
    }
}

fn is_action_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn is_table_name(value: &str) -> bool {
    match value.split_once('.') {
        Some((schema, table)) => {
            !table.contains('.') && is_table_part(schema) && is_table_part(table)
        }
        None => false,
    }
}

fn is_table_part(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn define_event_generates_idempotent_hook() {
        let define = DefineEvent {
            event_name: "market_data_changed".to_string(),
            on_table: "raw.market_data_bar".to_string(),
            hook_name: "emit.market_data_changed".to_string(),
            transaction_mode: TransactionMode::PostCommit,
        };
        let hook = define.compile_hook();

        assert_eq!(hook.name, "emit.market_data_changed");
        assert_eq!(
            define.idempotency_key(),
            "raw.market_data_bar:market_data_changed:emit.market_data_changed"
        );
    }

    #[test]
    fn rejects_unsafe_define_event_parts() {
        let define = DefineEvent {
            event_name: "market_data_changed".to_string(),
            on_table: "raw.market_data_bar;drop".to_string(),
            hook_name: "emit.market_data_changed".to_string(),
            transaction_mode: TransactionMode::PostCommit,
        };

        assert_eq!(
            define.try_compile_hook(),
            Err(DefineError::InvalidTableName)
        );
    }

    #[test]
    fn validate_rejects_each_bad_component() {
        let ok = DefineEvent {
            event_name: "market_data_changed".to_string(),
            on_table: "raw.market_data_bar".to_string(),
            hook_name: "emit.market_data_changed".to_string(),
            transaction_mode: TransactionMode::PostCommit,
        };
        assert_eq!(ok.validate(), Ok(()));

        // Bad event name (whitespace is not an allowed action-name character).
        assert_eq!(
            DefineEvent {
                event_name: "bad name".to_string(),
                ..ok.clone()
            }
            .validate(),
            Err(DefineError::InvalidEventName)
        );
        // Empty hook name is rejected (event + table still valid).
        assert_eq!(
            DefineEvent {
                hook_name: String::new(),
                ..ok.clone()
            }
            .validate(),
            Err(DefineError::InvalidHookName)
        );
        // on_table must be exactly schema.table: single-part and three-part rejected.
        assert_eq!(
            DefineEvent {
                on_table: "public".to_string(),
                ..ok.clone()
            }
            .validate(),
            Err(DefineError::InvalidTableName)
        );
        assert_eq!(
            DefineEvent {
                on_table: "a.b.c".to_string(),
                ..ok
            }
            .validate(),
            Err(DefineError::InvalidTableName)
        );
    }
}
