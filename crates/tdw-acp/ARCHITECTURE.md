# tdw-acp architecture

A single-module crate (`src/lib.rs`): the Agent Client Protocol message types and
their inbound validation.

## Module map

| Item | Role |
|------|------|
| `AcpServerInfo` | Handshake/capability descriptor (with a `Default`). |
| `AcpRequest` | `Initialize` / `SubmitOp` / `ResolveApproval` (serde `tag = "type"`, snake_case). |
| `AcpResponse` | `Initialized` / `Event` / `Error` (serde-tagged). |
| `AcpValidationError` | `EmptyField` / `UnsafeField` / `InvalidApprovalDecision` / `InvalidQuery`. |
| `validate_request` | The inbound guard, dispatching by request kind. |
| `parse_approval_decision` | String → `tdw-protocol::ApprovalDecision`. |
| `validate_op` / `validate_token` / `validate_display_text` / `validate_read_only_sql` (private) | The field-level guards. |

## Boundary-validation contract

ACP is the trust boundary between an external client and the daemon, so the
design rule is: **every inbound `AcpRequest` is fully validated before it is
acted on.** `validate_request` is exhaustive over the request enum and, for
`SubmitOp`, exhaustive over the protocol `Op` enum — there is no "default accept".

### Field guards

- **`validate_token`** (identifiers: providers, endpoints, tool names, stream
  ids, permission ids): non-empty, no control characters, none of `/ \ ; | &`,
  and no `..` traversal. This is what blocks `../approval` and injection-style
  values.
- **`validate_display_text`** (human-facing strings: messages, symbols, reasons):
  non-empty and free of control characters (but spaces/punctuation allowed).
- **`validate_read_only_sql`** (`RunQuery`): the same posture as `tdw-exec` —
  single statement, must start with `SELECT`, no comment markers (`--`, `/*`,
  `*/`) or mutating keywords (` drop `, ` delete `, ` insert `, ` update `).
- **`ResolveApproval`** additionally constructs a `tdw-protocol::PermissionId`
  (so an otherwise-safe token that isn't a valid id is rejected) and requires the
  decision to parse.

### Approval-decision normalization

`parse_approval_decision` trims, replaces `-` with `_`, and lowercases before
matching, so `Allow-Once`, `allow_once`, and `ALLOW_ONCE` all map to
`ApprovalDecision::AllowOnce`. It accepts both `always_allow` and `allow_always`
spellings, and `deny`; anything else is `InvalidApprovalDecision`.

## Message shapes

Requests and responses are serde-tagged enums (`#[serde(tag = "type",
rename_all = "snake_case")]`), so the JSON wire form is self-describing
(`"type": "submit_op"`, nested `"op": { "type": "append_user_message" }`) and
round-trips. `AcpResponse::Event` wraps a `tdw-protocol::EventMsg`, which is how
daemon events reach the client over ACP.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Validate at the boundary** — exhaustive request/op validation, fail-closed.
- **No multi-statement / mutating SQL**, **no traversal**, **no unknown approval
  decisions**.
- **Typed, round-tripping wire shapes.**
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
