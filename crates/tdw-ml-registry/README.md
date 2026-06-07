# tdw-ml-registry

In-memory registry of ML model registrations (id → metadata + artifact URI).

## Purpose

[`ModelRegistry`] is a small, dependency-free store of [`ModelRegistration`]
records, keyed by `model_id`. It validates each registration (id shape, version,
artifact URI scheme, owner) and rejects duplicates, giving the daemon a single
checked place to track which models exist and where their artifacts live.

Core surface:

- [`ModelKind`] — `Embedding` / `LanguageModel` / `Classifier` / `Reranker`.
- [`ModelRegistration`] — `{ model_id, kind, version, artifact_uri, owner }`.
- [`ModelRegistryError`] — `InvalidModelId`, `InvalidVersion`,
  `InvalidArtifactUri`, `InvalidOwner`, `DuplicateModel`.
- [`ModelRegistry`] — `register`, `get`, `model_ids`.
- [`validate_registration`] — the field checks.

## Feature flags

None. The crate has no dependencies.

## Environment variables

None.

## Quickstart

```rust
use tdw_ml_registry::{ModelKind, ModelRegistration, ModelRegistry};

let mut registry = ModelRegistry::default();
registry.register(ModelRegistration {
    model_id: "embedding/local-hash".to_string(),
    kind: ModelKind::Embedding,
    version: "0.1.0".to_string(),
    artifact_uri: "s3://models/local-hash".to_string(),
    owner: "tdw-ml-registry".to_string(),
})?;
assert_eq!(registry.model_ids(), vec!["embedding/local-hash".to_string()]);
# Ok::<(), tdw_ml_registry::ModelRegistryError>(())
```

Validation: `model_id` is `[A-Za-z0-9._/-]` with no `//`, no empty/`.`/`..`
segments; `artifact_uri` must be `s3://`, `https://`, or `file://`, with no `..`;
`version` and `owner` must be non-empty and control-free; re-registering an id is
`DuplicateModel`.

## Example

```text
cargo run --example tdw_ml_registry_basic -p tdw-ml-registry
```

`examples/basic.rs` registers a model, reads it back, and shows the
path-traversal and duplicate rejections — all in-memory.
