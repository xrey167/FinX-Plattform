# tdw-udf-python

Python UDF module contract + static sandbox validation for the TDW daemon.

## Purpose

`tdw-udf-python` defines the Python UDF module shape and a **static** validator
that rejects unsafe source before any execution: filesystem (`open(`,
`import pathlib`), process (`import subprocess`, `os.system`), and network
(`import socket`, `requests.`) access are denied. It does not embed a Python
interpreter — it is the validation/contract layer the daemon checks first.

Core surface:

- [`PythonUdfModule`] — `{ module_name, function_name, source }`.
- [`PythonUdfError`] — `EmptyModuleName`, `InvalidFunctionName`, `EmptySource`,
  `FilesystemAccessDenied`, `ProcessAccessDenied`, `NetworkAccessDenied`.
- [`validate_module`] — the static checks.
- Constants: `CRATE_NAME`, `RUNTIME_NAME = "python"`.

## Feature flags

None. The crate has no dependencies.

## Environment variables

None.

## Quickstart

```rust
use tdw_udf_python::{PythonUdfModule, validate_module};

let module = PythonUdfModule {
    module_name: "tdw-udf-python".to_string(),
    function_name: "upper".to_string(),
    source: "def upper(input):\n    return input.upper()\n".to_string(),
};
validate_module(&module)?;
# Ok::<(), tdw_udf_python::PythonUdfError>(())
```

`function_name` must be a Python identifier (leading letter/`_`, then
alphanumeric/`_`).

## Example

```text
cargo run --example tdw_udf_python_basic -p tdw-udf-python
```

`examples/basic.rs` validates a safe module and shows the filesystem, process,
and network denials.
