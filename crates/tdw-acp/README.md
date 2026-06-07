# tdw-acp

The Agent Client Protocol (ACP) boundary: typed request/response messages
between a client and the daemon, with strict validation of every inbound request.

`tdw-acp` defines the wire shapes a client uses to talk to the platform —
initialize, submit a protocol `Op`, resolve an approval — and the corresponding
responses (initialized, event, error). Its central job is `validate_request`:
nothing crosses the boundary without passing identifier, display-text, SQL, and
approval-decision guards.

## What it provides

- `AcpServerInfo` — capability/handshake info (name, protocol version, streaming
  / approvals support).
- `AcpRequest` — `Initialize` / `SubmitOp` / `ResolveApproval` (serde-tagged).
- `AcpResponse` — `Initialized` / `Event` / `Error`.
- `validate_request(request)` — the inbound guard.
- `parse_approval_decision(value)` — normalize an approval string to
  `tdw-protocol::ApprovalDecision`.
- `AcpValidationError` (impls `Display` + `std::error::Error`).

## Feature flags

None. Depends on `serde`, `serde_json`, and `tdw-protocol`.

## Quickstart

```rust
use tdw_acp::{validate_request, AcpRequest, AcpValidationError};

let init = AcpRequest::Initialize { client_name: "tdw-cli".to_string() };
assert!(validate_request(&init).is_ok());

// Path-traversal in a permission id is rejected.
let bad = AcpRequest::ResolveApproval {
    permission_id: "../approval".to_string(),
    decision: "allow_once".to_string(),
};
assert_eq!(
    validate_request(&bad),
    Err(AcpValidationError::UnsafeField { field: "permission_id" }),
);
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-acp --example tdw_acp_basic
```

## Validation contract

`validate_request` dispatches by request kind:

- **`Initialize`** — `client_name` must be a safe token.
- **`SubmitOp`** — the embedded `Op` is validated field-by-field: tokens
  (`provider`, `endpoint`, `tool_name`, `stream_id`, …) must be safe; display
  text (messages, symbols, reasons) must be non-empty and control-char-free; a
  `RunQuery` SQL must pass the read-only guard; `CompactContext` must have a
  positive token budget.
- **`ResolveApproval`** — `permission_id` must be a safe token *and* a valid
  `tdw-protocol::PermissionId`, and `decision` must parse via
  `parse_approval_decision` (`allow_once`, `always_allow` / `allow_always`,
  `deny`; case- and `-`/`_`-insensitive).

The token guard rejects empty values, control characters, `/ \ ; | &`, and `..`
traversal. The read-only SQL guard mirrors `tdw-exec`: single statement, must
start with `SELECT`, no comment markers or mutating keywords.

## Invariants

- `#![forbid(unsafe_code)]`.
- **Validate at the boundary.** Every inbound request is fully checked before the
  daemon acts on it; unsafe tokens, traversal, multi-statement or mutating SQL,
  and unknown approval decisions are all rejected.
- **Typed wire shapes.** Requests/responses are serde-tagged enums, so the
  protocol is explicit and round-trips.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
