//! Integration tests for the opt-in, append-only hash-chained receipt log.
//!
//! These reuse the `tool_with_impl` / `registry_with` / `echo_handler` fixture
//! pattern from the crate's unit tests and `examples/basic.rs`.

#![forbid(unsafe_code)]

use serde_json::{Value, json};
use tdw_agent::{
    Adaptivity, EntityMeta, Origin, Registry, RegistryEntity, Source, Tier, Tool, ToolEffect,
    ToolImplementation,
};
use tdw_tool_exec::{ChainBreak, ChainStatus, GENESIS_PREV_HASH, ReceiptLog, ToolExecutor};

#[allow(clippy::needless_pass_by_value)]
fn echo_handler(input: Value) -> tdw_tools::Result<Value> {
    Ok(json!({ "echoed": input }))
}

fn tool_with_impl(name: &str, implementation: ToolImplementation) -> Tool {
    Tool {
        meta: EntityMeta::new(
            name,
            name,
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::None,
            false,
        )
        .with_title(name)
        .with_description("receipt-log test fixture"),
        input_schema: json!({ "type": "object" }),
        output_schema: None,
        effect: ToolEffect::ReadOnly,
        idempotent: true,
        open_world: false,
        implementation,
    }
}

fn registry_with(tool: &Tool) -> Registry {
    Registry::from_resources([tool
        .to_resource()
        .unwrap_or_else(|error| panic!("tool resource: {error}"))])
    .unwrap_or_else(|error| panic!("registry should build: {error}"))
}

const TOOL: &str = "tool.exec.builtin";
const HANDLER: &str = "tool.exec.builtin.handler";

/// An executor with `HANDLER` registered to `echo_handler`.
fn echo_executor() -> ToolExecutor {
    ToolExecutor::new()
        .with_builtin(HANDLER, echo_handler)
        .unwrap_or_else(|error| panic!("builtin should register: {error}"))
}

fn builtin_registry() -> Registry {
    let tool = tool_with_impl(
        TOOL,
        ToolImplementation::Builtin {
            handler: HANDLER.to_string(),
        },
    );
    registry_with(&tool)
}

#[test]
fn disabled_by_default_records_nothing() {
    let registry = builtin_registry();
    let executor = echo_executor();

    executor
        .execute(&registry, TOOL, &json!({ "ping": 1 }))
        .unwrap_or_else(|error| panic!("execute: {error}"));

    assert!(
        executor.receipts().is_none(),
        "receipts must be None without .with_receipts()"
    );
}

#[test]
fn enabled_appends_one_receipt_per_successful_execute() {
    let registry = builtin_registry();
    let executor = echo_executor().with_receipts();

    executor
        .execute(&registry, TOOL, &json!({ "n": 1 }))
        .unwrap_or_else(|error| panic!("execute 1: {error}"));
    executor
        .execute(&registry, TOOL, &json!({ "n": 2 }))
        .unwrap_or_else(|error| panic!("execute 2: {error}"));

    let log = executor.receipts().expect("receipts enabled");
    assert_eq!(log.len(), 2);
    let receipts = log.snapshot();
    assert_eq!(receipts[0].seq, 0);
    assert_eq!(receipts[1].seq, 1);
    assert_eq!(receipts[0].tool, TOOL);
}

#[test]
fn genesis_prev_hash_and_linkage() {
    let registry = builtin_registry();
    let executor = echo_executor().with_receipts();

    executor
        .execute(&registry, TOOL, &json!({ "n": 1 }))
        .unwrap_or_else(|error| panic!("execute 1: {error}"));
    executor
        .execute(&registry, TOOL, &json!({ "n": 2 }))
        .unwrap_or_else(|error| panic!("execute 2: {error}"));

    let receipts = executor.receipts().expect("enabled").snapshot();
    assert_eq!(receipts[0].prev_hash, GENESIS_PREV_HASH);
    assert_eq!(receipts[1].prev_hash, receipts[0].this_hash);
}

#[test]
fn this_hash_recomputes_and_chain_is_valid() {
    let registry = builtin_registry();
    let executor = echo_executor().with_receipts();

    for n in 0..3 {
        executor
            .execute(&registry, TOOL, &json!({ "n": n }))
            .unwrap_or_else(|error| panic!("execute {n}: {error}"));
    }

    let receipts = executor.receipts().expect("enabled").snapshot();
    assert_eq!(receipts.len(), 3);
    assert_eq!(ReceiptLog::verify(&receipts), ChainStatus::Valid);
}

#[test]
fn tampered_field_breaks_chain_thishash() {
    let registry = builtin_registry();
    let executor = echo_executor().with_receipts();
    for n in 0..3 {
        executor
            .execute(&registry, TOOL, &json!({ "n": n }))
            .unwrap_or_else(|error| panic!("execute {n}: {error}"));
    }

    let mut receipts = executor.receipts().expect("enabled").snapshot();
    // Mutate a field without recomputing this_hash: the recompute must mismatch.
    receipts[1].args_hash = receipts[1].args_hash.wrapping_add(1);

    assert_eq!(
        ReceiptLog::verify(&receipts),
        ChainStatus::Broken {
            seq: 1,
            reason: ChainBreak::ThisHash,
        }
    );
}

#[test]
fn tampered_prev_hash_breaks_chain() {
    let registry = builtin_registry();
    let executor = echo_executor().with_receipts();
    for n in 0..3 {
        executor
            .execute(&registry, TOOL, &json!({ "n": n }))
            .unwrap_or_else(|error| panic!("execute {n}: {error}"));
    }

    let mut receipts = executor.receipts().expect("enabled").snapshot();
    receipts[1].prev_hash = receipts[1].prev_hash.wrapping_add(99);

    assert_eq!(
        ReceiptLog::verify(&receipts),
        ChainStatus::Broken {
            seq: 1,
            reason: ChainBreak::PrevHash,
        }
    );
}

#[test]
fn reordered_seq_breaks_chain() {
    let registry = builtin_registry();
    let executor = echo_executor().with_receipts();
    for n in 0..3 {
        executor
            .execute(&registry, TOOL, &json!({ "n": n }))
            .unwrap_or_else(|error| panic!("execute {n}: {error}"));
    }

    let mut receipts = executor.receipts().expect("enabled").snapshot();
    receipts.swap(0, 1);

    match ReceiptLog::verify(&receipts) {
        ChainStatus::Broken {
            reason: ChainBreak::Seq,
            ..
        } => {}
        other => panic!("expected Seq break, got {other:?}"),
    }
}

#[test]
fn empty_chain_is_valid() {
    assert_eq!(ReceiptLog::verify(&[]), ChainStatus::Valid);
    assert_eq!(ReceiptLog::new().last_hash(), GENESIS_PREV_HASH);
    assert!(ReceiptLog::new().is_empty());
}

#[test]
fn args_hash_is_canonical_order_independent() {
    let registry = builtin_registry();
    let executor = echo_executor().with_receipts();

    // Two args with identical entries inserted in different orders.
    let first: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).expect("parse a");
    let second: Value = serde_json::from_str(r#"{"b":2,"a":1}"#).expect("parse b");

    executor
        .execute(&registry, TOOL, &first)
        .unwrap_or_else(|error| panic!("execute 1: {error}"));
    executor
        .execute(&registry, TOOL, &second)
        .unwrap_or_else(|error| panic!("execute 2: {error}"));

    let receipts = executor.receipts().expect("enabled").snapshot();
    assert_eq!(
        receipts[0].args_hash, receipts[1].args_hash,
        "canonicalization must make key order irrelevant"
    );
}

#[test]
fn different_args_or_results_differ() {
    let registry = builtin_registry();
    let executor = echo_executor().with_receipts();

    // echo_handler reflects its input, so different args also yield different results.
    executor
        .execute(&registry, TOOL, &json!({ "k": "one" }))
        .unwrap_or_else(|error| panic!("execute 1: {error}"));
    executor
        .execute(&registry, TOOL, &json!({ "k": "two" }))
        .unwrap_or_else(|error| panic!("execute 2: {error}"));

    let receipts = executor.receipts().expect("enabled").snapshot();
    assert_ne!(
        receipts[0].args_hash, receipts[1].args_hash,
        "distinct args must hash differently"
    );
    assert_ne!(
        receipts[0].result_hash, receipts[1].result_hash,
        "distinct results must hash differently"
    );
}

#[test]
fn failed_execute_is_not_recorded() {
    let tool = tool_with_impl("tool.exec.unbound", ToolImplementation::Unbound);
    let registry = registry_with(&tool);
    let executor = ToolExecutor::new().with_receipts();

    let error = executor.execute(&registry, "tool.exec.unbound", &json!({}));
    assert!(error.is_err(), "unbound tool must error");

    let log = executor.receipts().expect("enabled");
    assert_eq!(log.len(), 0, "failed executions are not receipts");
}
