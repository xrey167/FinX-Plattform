# tdw-provider-huggingface

Inference provider for the **Hugging Face Inference API** (text generation).
Exposes a typed query/result model and a `tdw_core::Fetcher` implementation for
the `/models/{model_id}` text-generation endpoint.

- **Vendor:** Hugging Face — Inference API
- **Base URL:** `https://api-inference.huggingface.co`
- **Endpoint:** `text_generation` — `POST /models/{model_id}`
- **Auth:** bearer token in the `Authorization` header.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `HuggingFaceHttpTextGenerationFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`. |

With `http` off the typed models, model-id validation (path-traversal safe),
and the `text_generation_request` request-contract builder are available with no
network dependencies.

## Environment variables

The token is read from the **first** of these that is set and non-empty:

| Variable                    | Required | Purpose |
| --------------------------- | -------- | ------- |
| `HF_TOKEN`                  | for live calls | Hugging Face access token (preferred). |
| `HUGGINGFACE_API_TOKEN`     | fallback | Alternative token env name. |
| `HF_API_TOKEN`              | fallback | Alternative token env name. |
| `TDW_HUGGINGFACE_LIVE`      | no       | Set to `1` to enable the live network integration test. |

The header name is exported as `AUTH_HEADER`.

## Quickstart

Offline (default features):

```rust
use tdw_provider_huggingface::{HuggingFaceTextGenerationQuery, text_generation_request};

let query = HuggingFaceTextGenerationQuery::new("gpt2", "Hello")?
    .with_max_new_tokens(8)?;
let request = text_generation_request(&query.model_id, /* token_present = */ true)?;
println!("{} {}", request.provider, request.path);
# Ok::<(), tdw_provider_huggingface::HuggingFaceProviderError>(())
```

Live HTTP (requires `--features http` and an HF token env):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_huggingface::HuggingFaceHttpTextGenerationFetcher;

let fetcher = HuggingFaceHttpTextGenerationFetcher::default();
let rows = fetcher
    .fetch(serde_json::json!({ "model_id": "gpt2", "inputs": "Hello", "max_new_tokens": 8 }),
           &Credentials::default())
    .await?;
```

## Example

```bash
cargo run -p tdw-provider-huggingface --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider env-var conventions and feature-gating model.
