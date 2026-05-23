# Session, Rollout, And Daemon Loop

G005 adds the durable runtime substrate:

- `tdw-session` owns SQLx/SQLite hot state for sessions, permission rules,
  pending approvals, and operation cost ledger entries.
- `tdw-rollout` owns append-only JSONL rollout records around
  `tdw-protocol::ReplayFrame`.
- `tdw-app-server` owns daemon endpoint metadata, the submission queue, event
  channel, and a hand-rolled `tokio::select!` loop that emits protocol events.
- `tdw-app-client` is a thin client wrapper over the submission handle and
  endpoint contract.

The local implementation uses in-process channels for tests. The public
`DaemonEndpoint` contract already distinguishes UDS from HTTP+SSE so later CLI,
TUI, MCP, and web shells can move to real transports without changing protocol
message types.
