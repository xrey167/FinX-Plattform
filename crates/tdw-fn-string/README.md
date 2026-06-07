# tdw-fn-string

A tiny, deterministic, allow-listed string-transformation pipeline.

`tdw-fn-string` applies an ordered list of safe string operations (`Trim`,
`Uppercase`, `Lowercase`, `Replace`) to an input string. It is the building block
for normalization steps (e.g. canonicalizing a symbol) where the set of allowed
operations must be fixed and the patterns must be validated for safety.

## What it provides

- `StringFn` — one operation: `Trim`, `Uppercase`, `Lowercase`, or
  `Replace { from, to }`.
- `StringPipeline` — `{ name, steps }`.
- `apply_pipeline(input, pipeline)` — validate then run, returning
  `Result<String, StringFnError>`.
- `validate_pipeline(pipeline)` — validate without running.
- `StringFnError` — `InvalidPipelineName` / `EmptyPipeline` / `EmptyPattern` /
  `UnsafePattern`.

## Feature flags

None. The crate has **no dependencies**.

## Quickstart

```rust
use tdw_fn_string::{apply_pipeline, StringFn, StringPipeline};

let pipeline = StringPipeline {
    name: "normalize-symbol".to_string(),
    steps: vec![
        StringFn::Trim,
        StringFn::Replace { from: " ".to_string(), to: "_".to_string() },
        StringFn::Uppercase,
    ],
};

assert_eq!(apply_pipeline(" aapl equity ", &pipeline), Ok("AAPL_EQUITY".to_string()));
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-fn-string --example tdw_fn_string_basic
```

## Safety rules

`validate_pipeline` (run by `apply_pipeline` first) rejects:

- a pipeline `name` that is not `[A-Za-z0-9_-]+` → `InvalidPipelineName`;
- an empty `steps` list → `EmptyPipeline`;
- a `Replace` with an empty `from` → `EmptyPattern`;
- a `Replace` whose `from` or `to` contains a control character or a shell
  metacharacter (`;`, `|`, `` ` ``) → `UnsafePattern`.

`Uppercase` / `Lowercase` are ASCII-only (`to_ascii_uppercase` /
`to_ascii_lowercase`), keeping the transform deterministic and locale-independent.

## Invariants

- `#![forbid(unsafe_code)]`.
- **Validate before transform** — patterns are checked before any replacement.
- **Deterministic** — pure function of input + pipeline; ASCII case folding.
- **No shell/control injection** in `Replace` patterns.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
