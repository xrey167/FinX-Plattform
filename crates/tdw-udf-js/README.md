# tdw-udf-js

JavaScript UDF module contract + static sandbox validation for the TDW daemon.

## Purpose

`tdw-udf-js` defines the JavaScript UDF module shape and a **static** validator
that rejects unsafe source before any execution: dynamic `import()` and network
access (`fetch(`, `XMLHttpRequest`) are denied. It does not embed a JS engine —
it is the validation/contract layer the daemon checks before handing a module to
a runtime.

Core surface:

- [`JavaScriptUdfModule`] — `{ module_name, entrypoint, source }`.
- [`JavaScriptUdfError`] — `EmptyModuleName`, `InvalidEntrypoint`, `EmptySource`,
  `DynamicImportDenied`, `NetworkAccessDenied`.
- [`validate_module`] — the static checks.
- Constants: `CRATE_NAME`, `RUNTIME_NAME = "javascript"`.

## Feature flags

None. The crate has no dependencies.

## Environment variables

None.

## Quickstart

```rust
use tdw_udf_js::{JavaScriptUdfModule, validate_module};

let module = JavaScriptUdfModule {
    module_name: "tdw-udf-js".to_string(),
    entrypoint: "tdw.transform.upper".to_string(),
    source: "export function upper(input) { return input.toUpperCase(); }".to_string(),
};
validate_module(&module)?;
# Ok::<(), tdw_udf_js::JavaScriptUdfError>(())
```

The `entrypoint` must be a dotted identifier path (`[A-Za-z0-9_$]`, dot-separated,
no empty segments).

## Example

```text
cargo run --example tdw_udf_js_basic -p tdw-udf-js
```

`examples/basic.rs` validates a safe module and shows the dynamic-import and
network-access denials.
