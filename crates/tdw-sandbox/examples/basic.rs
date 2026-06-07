//! Offline `tdw-sandbox` example: run a fixture UDF through `LocalUdfSandbox`
//! and show the capability gate denying a network-requesting request.
//!
//! Run with (default build — no `udf-wasm` feature needed; the `Wasm` runtime
//! variant routes to the deterministic `tdw-udf` fixture interpreter):
//!
//! ```sh
//! cargo run -p tdw-sandbox --example tdw_sandbox_basic
//! ```

#![forbid(unsafe_code)]

use tdw_sandbox::{LocalUdfSandbox, SandboxError, SandboxRuntime, UdfRequest};
use tdw_udf::UdfRuntime;

/// Build a request for the fixture `upper` UDF over `input`.
fn upper_request(input: &str, allow_network: bool) -> UdfRequest {
    UdfRequest {
        name: "upper".to_string(),
        runtime: UdfRuntime::Wasm,
        // The fixture interpreter dispatches by `name`; `source` only needs to
        // be non-empty to pass validation.
        source: "upper(input)".to_string(),
        input: input.to_string(),
        allow_network,
        allow_filesystem: false,
        wasm_limits: None,
    }
}

fn main() {
    let sandbox = LocalUdfSandbox;
    println!("sandbox runtime: {}", sandbox.runtime_name());

    // Happy path: the fixture `upper` UDF uppercases its input.
    let response = sandbox
        .run(upper_request("aapl", false))
        .expect("fixture udf runs");
    assert_eq!(response.output, "AAPL");
    assert_eq!(response.runtime, UdfRuntime::Wasm);
    println!("upper(\"aapl\") = {:?}", response.output);

    // Capability gate: requesting network is denied before any UDF runs.
    let denied = sandbox.run(upper_request("aapl", true));
    assert_eq!(denied, Err(SandboxError::CapabilityDenied("network")));
    println!("network capability denied at the boundary");

    // Validation gate: an empty source is rejected before dispatch.
    let mut empty_source = upper_request("aapl", false);
    empty_source.source = "   ".to_string();
    assert_eq!(
        sandbox.run(empty_source),
        Err(SandboxError::InvalidRequest("source")),
    );
    println!("empty source rejected before dispatch");
}
