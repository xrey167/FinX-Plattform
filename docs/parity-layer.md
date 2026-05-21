# Parity Layer

Layer C parity features are implemented as small contracts over the existing
runtime and event spine:

- `tdw-snapshot`: snapshot versions and time-travel lookup.
- `tdw-bus`, `tdw-cdc`, and `tdw-replay`: streams, live-query evidence, CDC
  offsets, and replay dry runs.
- `tdw-graph` and `tdw-spatial`: graph traversal/cycle checks and spatial
  bounding-box predicates.
- `tdw-stage`, `tdw-pipe`, and `tdw-table-format`: stage/COPY plans, pipe
  offsets, and Iceberg/Delta-style manifest checksum verification.
- `tdw-udf`: sandboxed UDF definitions with denied network/filesystem
  capabilities.
- `tdw-auth` and `tdw-auth-oidc`: role policy and JWT/JWKS claim validation.
- `tdw-define` and `tdw-mask`: declarative event hook generation and masking as
  a sync filter hook.

`tdw-service-api::parity_layer_sample` wires these pieces together for a single
runtime smoke path. The service binary prints this sample as `parity=...`.
