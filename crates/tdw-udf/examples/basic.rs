//! Offline `tdw-udf` example: validate a UDF definition and evaluate the builtin
//! transforms, then show the deny-by-default capability path.
//!
//! No network and no filesystem — the sandbox denies both by contract.
//!
//! ```text
//! cargo run --example tdw_udf_basic -p tdw-udf
//! ```

use tdw_udf::{UdfDefinition, UdfError, UdfRuntime, evaluate, validate_definition};

fn main() {
    let definition = UdfDefinition {
        name: "upper".to_string(),
        runtime: UdfRuntime::Wasm,
        source: "(input) => input.toUpperCase()".to_string(),
        allow_network: false,
        allow_filesystem: false,
    };

    validate_definition(&definition).expect("definition should validate");

    // Builtin transforms.
    let upper = evaluate(&definition, "aapl").expect("upper should evaluate");
    assert_eq!(upper, "AAPL");
    let identity_def = UdfDefinition {
        name: "identity".to_string(),
        ..definition.clone()
    };
    let identity = evaluate(&identity_def, "AAPL").expect("identity should evaluate");
    assert_eq!(identity, "AAPL");
    println!("upper(aapl)    = {upper}");
    println!("identity(AAPL) = {identity}");

    // Deny-by-default: requesting network is a hard error.
    let networked = UdfDefinition {
        allow_network: true,
        ..definition
    };
    assert_eq!(
        evaluate(&networked, "aapl"),
        Err(UdfError::CapabilityDenied("network"))
    );
    println!("network capability is denied by the sandbox, as expected");
}
