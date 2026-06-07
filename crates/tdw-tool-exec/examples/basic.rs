//! Offline `tdw-tool-exec` example: register a `Builtin` tool in a `tdw-agent`
//! registry and dispatch it through `ToolExecutor`, then show an `Unbound` tool
//! returning the unbound error.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-tool-exec --example tdw_tool_exec_basic
//! ```

#![forbid(unsafe_code)]

use serde_json::{Value, json};
use tdw_agent::{
    Adaptivity, EntityMeta, Origin, Registry, RegistryEntity, Source, Tier, Tool, ToolEffect,
    ToolImplementation,
};
use tdw_tool_exec::{ExecError, ToolExecutor};

/// A builtin handler: echoes its input back under `echoed`.
/// The `fn(Value) -> tdw_tools::Result<Value>` shape is fixed by `ToolHandler`.
#[allow(clippy::needless_pass_by_value)]
fn echo_handler(input: Value) -> tdw_tools::Result<Value> {
    Ok(json!({ "echoed": input }))
}

/// Build a `tdw-agent` `Tool` resource with the given implementation.
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
        .with_description("tool-exec example fixture"),
        input_schema: json!({ "type": "object" }),
        output_schema: None,
        effect: ToolEffect::ReadOnly,
        idempotent: true,
        open_world: false,
        implementation,
    }
}

fn registry_with(tool: &Tool) -> Registry {
    Registry::from_resources([tool.to_resource().expect("tool resource")]).expect("registry builds")
}

fn main() {
    // A builtin tool resolves its handler and runs it in-process.
    let builtin = tool_with_impl(
        "demo.tool",
        ToolImplementation::Builtin {
            handler: "demo.tool.handler".to_string(),
        },
    );
    let registry = registry_with(&builtin);
    let executor = ToolExecutor::new()
        .with_builtin("demo.tool.handler", echo_handler)
        .expect("register builtin handler");

    let outcome = executor
        .execute(&registry, "demo.tool", &json!({ "ping": true }))
        .expect("builtin executes");
    assert_eq!(outcome.structured["echoed"], json!({ "ping": true }));
    println!("builtin dispatched: {}", outcome.structured);

    // An unbound tool surfaces the unbound error (the MCP -32601 path).
    let unbound = tool_with_impl("demo.unbound", ToolImplementation::Unbound);
    let unbound_registry = registry_with(&unbound);
    let error = ToolExecutor::new()
        .execute(&unbound_registry, "demo.unbound", &json!({}))
        .expect_err("unbound tool errors");
    assert!(matches!(error, ExecError::Unbound));
    println!("unbound tool -> {error}");

    // A tool that isn't registered at all is ToolNotFound.
    let missing = ToolExecutor::new().execute(&Registry::new(), "demo.nope", &json!({}));
    assert!(matches!(missing, Err(ExecError::ToolNotFound(_))));
    println!("missing tool -> {missing:?}");
}
