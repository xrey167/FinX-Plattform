# Thin Clients And Replay

G007 moves user-facing shells toward protocol events:

- `tdw-exec` returns `EventMsg` streams for headless execution.
- `tdw-tui` converts `EventMsg` values into ratatui `Line` values without
  depending on core runtime internals.
- `tdw-replay::ReplayEngine::from_rollout` summarizes append-only rollout
  records as protocol replay evidence.
- `tdw-service-api::client_event_sample` wires headless exec, TUI lines, and
  replay summaries into one service-facing path.
- `tdw-cli` and `tdw-mcp` now print the protocol-event evidence from the shared
  service API instead of embedding their own event logic.
