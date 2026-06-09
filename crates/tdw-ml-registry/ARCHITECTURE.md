# tdw-ml-registry — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`, no dependencies:

| Item | Role |
| --- | --- |
| `ModelKind` | The closed set of model kinds. |
| `ModelRegistration` | The registration record. |
| `ModelRegistryError` | The validation/duplicate error enum. |
| `ModelRegistry` | The in-memory store (`BTreeMap<String, ModelRegistration>`). |
| `validate_registration` | Field hygiene checks. |

## Registration contract

`ModelRegistry::register(model)`:

1. `validate_registration(&model)` (see below).
2. Reject a `model_id` already present → `DuplicateModel`.
3. Insert keyed by `model_id`.

`get(id)` borrows by id; `model_ids()` returns ids in `BTreeMap` (sorted) order,
which keeps listings deterministic.

## Validation rules

`validate_registration`:

- **model_id** (`is_model_id`): non-empty, only `[A-Za-z0-9._/-]`, no `//`, and no
  segment that is empty / `.` / `..` (path-traversal safe even though ids look
  path-like).
- **version**: non-empty after trim, no control characters.
- **artifact_uri**: must start with `s3://`, `https://`, or `file://`; must not
  contain `..`; no control characters.
- **owner**: non-empty after trim, no control characters.

## Offline test design

Pure in-memory unit tests: a valid registration round-trips (`register` →
`get` → `model_ids`), a traversal-style id (`../secret`) is rejected
(`InvalidModelId`), and re-registering an existing id is `DuplicateModel`. No
async, no I/O, no network.
