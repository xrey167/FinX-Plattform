#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_event::EventEnvelope;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionMode {
    InTransaction,
    PostCommit,
    Rollback,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSpec {
    pub name: String,
    pub order: i32,
    pub enabled: bool,
    pub transaction_mode: TransactionMode,
    pub max_depth: u8,
}

impl HookSpec {
    pub fn new(name: impl Into<String>, order: i32, transaction_mode: TransactionMode) -> Self {
        Self {
            name: name.into(),
            order,
            enabled: true,
            transaction_mode,
            max_depth: 8,
        }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookOutcome {
    pub name: String,
    pub transaction_mode: TransactionMode,
    pub emitted_event_type: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HookError {
    #[error("hook recursion guard blocked {0}")]
    RecursionGuard(String),
    #[error("hook depth exceeded at {0}")]
    DepthExceeded(String),
}

#[derive(Clone, Debug, Default)]
pub struct HookRegistry {
    hooks: Vec<HookSpec>,
    active: BTreeSet<String>,
}

impl HookRegistry {
    pub fn register(&mut self, hook: HookSpec) {
        self.hooks.push(hook);
        self.hooks.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then(left.name.cmp(&right.name))
        });
    }

    pub fn disable(&mut self, name: &str) {
        for hook in &mut self.hooks {
            if hook.name == name {
                hook.enabled = false;
            }
        }
    }

    pub fn execute(
        &mut self,
        envelope: &EventEnvelope<Value>,
    ) -> Result<Vec<HookOutcome>, HookError> {
        let mut outcomes = Vec::new();
        for hook in self.hooks.clone().into_iter().filter(|hook| hook.enabled) {
            if envelope.depth >= hook.max_depth {
                return Err(HookError::DepthExceeded(hook.name));
            }
            if envelope.event_type == hook.name || self.active.contains(&hook.name) {
                return Err(HookError::RecursionGuard(hook.name));
            }
            self.active.insert(hook.name.clone());
            outcomes.push(HookOutcome {
                emitted_event_type: format!("hook.{}", hook.name),
                name: hook.name.clone(),
                transaction_mode: hook.transaction_mode,
            });
            self.active.remove(&hook.name);
        }
        Ok(outcomes)
    }

    pub fn hook_names(&self) -> Vec<String> {
        self.hooks.iter().map(|hook| hook.name.clone()).collect()
    }
}

#[macro_export]
macro_rules! event_hook {
    ($name:expr, $order:expr, $mode:expr) => {
        $crate::HookSpec::new($name, $order, $mode)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_event::sample_event;

    #[test]
    fn hooks_execute_in_deterministic_order_and_skip_disabled() {
        let mut registry = HookRegistry::default();
        registry.register(event_hook!("post", 20, TransactionMode::PostCommit));
        registry.register(event_hook!("sync", 10, TransactionMode::InTransaction));
        registry.register(event_hook!("off", 15, TransactionMode::Rollback).disabled());

        let outcomes = registry
            .execute(&sample_event("service"))
            .unwrap_or_else(|error| panic!("hooks should execute: {error}"));

        assert_eq!(outcomes[0].name, "sync");
        assert_eq!(outcomes[1].name, "post");
        assert_eq!(outcomes.len(), 2);
    }

    #[test]
    fn recursion_guard_rejects_self_emitting_hook() {
        let mut registry = HookRegistry::default();
        registry.register(event_hook!(
            "ingress.received",
            1,
            TransactionMode::InTransaction
        ));

        assert_eq!(
            registry.execute(&sample_event("service")),
            Err(HookError::RecursionGuard("ingress.received".to_string()))
        );
    }
}
