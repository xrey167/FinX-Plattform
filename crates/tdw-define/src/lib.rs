#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_hooks::{HookSpec, TransactionMode};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefineEvent {
    pub event_name: String,
    pub on_table: String,
    pub hook_name: String,
    pub transaction_mode: TransactionMode,
}

impl DefineEvent {
    pub fn compile_hook(&self) -> HookSpec {
        HookSpec::new(self.hook_name.clone(), 100, self.transaction_mode)
    }

    pub fn idempotency_key(&self) -> String {
        format!("{}:{}:{}", self.on_table, self.event_name, self.hook_name)
    }
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
}
