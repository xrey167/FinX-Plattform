# tdw-udf-python — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`, no dependencies:

| Item | Role |
| --- | --- |
| `PythonUdfModule` | `{ module_name, function_name, source }`. |
| `PythonUdfError` | The validation error enum. |
| `validate_module` | Static sandbox validation. |
| `CRATE_NAME` / `RUNTIME_NAME` | Identity constants (`RUNTIME_NAME = "python"`). |

## UDF runtime contract

The crate is the **static gate** for the Python runtime, not the interpreter.
The daemon validates a `PythonUdfModule` here before any interpreter sees it.

## Sandbox design (static analysis)

`validate_module` rejects, in order:

1. Empty `module_name` → `EmptyModuleName`.
2. `function_name` that is not a Python identifier → `InvalidFunctionName`
   (first char letter or `_`, rest alphanumeric or `_`).
3. Empty `source` → `EmptySource`.
4. `open(` or `import pathlib` → `FilesystemAccessDenied`.
5. `import subprocess` or `os.system` → `ProcessAccessDenied`.
6. `import socket` or `requests.` → `NetworkAccessDenied`.

Three capability classes — filesystem, process, network — are each denied by a
conservative substring check. As with the JS gate, this is a pre-execution
filter that favours false-positives over letting an unsafe call through; it is
not a full Python parser.

## Offline test design

Pure unit tests over `validate_module`: the safe module passes, and a
filesystem (`open(`), process (`import subprocess`), and network
(`import socket`) sample are each rejected with the matching error. No
interpreter, no network, no filesystem.
