//! Offline `tdw-udf-js` example: validate a safe JavaScript UDF module and show
//! the static dynamic-import / network-access denials.
//!
//! No JS engine, no network — this is the static validation gate only.
//!
//! ```text
//! cargo run --example tdw_udf_js_basic -p tdw-udf-js
//! ```

use tdw_udf_js::{JavaScriptUdfError, JavaScriptUdfModule, RUNTIME_NAME, validate_module};

fn main() {
    let module = JavaScriptUdfModule {
        module_name: "tdw-udf-js".to_string(),
        entrypoint: "tdw.transform.upper".to_string(),
        source: "export function upper(input) { return input.toUpperCase(); }".to_string(),
    };

    validate_module(&module).expect("safe module should validate");
    println!("runtime: {RUNTIME_NAME}");
    println!("validated entrypoint: {}", module.entrypoint);

    // Network access is denied.
    let networked = JavaScriptUdfModule {
        source: "export async function f() { return fetch('https://example.com'); }".to_string(),
        ..module.clone()
    };
    assert_eq!(
        validate_module(&networked),
        Err(JavaScriptUdfError::NetworkAccessDenied)
    );

    // Dynamic import is denied.
    let dynamic = JavaScriptUdfModule {
        source: "import('fs')".to_string(),
        ..module
    };
    assert_eq!(
        validate_module(&dynamic),
        Err(JavaScriptUdfError::DynamicImportDenied)
    );
    println!("network access and dynamic import are denied, as expected");
}
