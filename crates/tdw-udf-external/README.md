# tdw-udf-external

External-command UDF contract + static validation for the TDW daemon.

## Purpose

`tdw-udf-external` defines the shape of an external-process UDF (a command run as
a subprocess) and a **static** validator that rejects unsafe command shapes
before any spawn: path components (`/`, `\`), shell control characters
(`;`, `&`, `|`, backtick, control chars), and unbounded timeouts are denied. It
does not spawn anything — it is the validation/contract layer the daemon checks
first.

Core surface:

- [`ExternalUdfCommand`] — `{ name, command, args, timeout_ms }`.
- [`ExternalUdfError`] — `EmptyName`, `InvalidCommand`, `InvalidArgument`,
  `InvalidTimeout`.
- [`validate_command`] — the static checks.
- Constants: `CRATE_NAME`, `RUNTIME_NAME = "external"`, `MAX_TIMEOUT_MS = 30_000`.

## Feature flags

None. The crate has no dependencies.

## Environment variables

None.

## Quickstart

```rust
use tdw_udf_external::{ExternalUdfCommand, validate_command};

let command = ExternalUdfCommand {
    name: "tdw-udf-external".to_string(),
    command: "tdw-udf-runner".to_string(),
    args: vec!["--runtime".to_string(), "wasm".to_string()],
    timeout_ms: 5_000,
};
validate_command(&command)?;
# Ok::<(), tdw_udf_external::ExternalUdfError>(())
```

`command` must be a bare program name (`[A-Za-z0-9_-]`, no path separators, no
shell control). `timeout_ms` must be in `1..=MAX_TIMEOUT_MS`.

## Example

```text
cargo run --example tdw_udf_external_basic -p tdw-udf-external
```

`examples/basic.rs` validates an allowlisted command and shows the path,
shell-injection, and timeout rejections.
