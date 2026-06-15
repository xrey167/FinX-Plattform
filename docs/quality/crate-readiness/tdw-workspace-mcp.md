# tdw-workspace-mcp Readiness Worksheet

Generated during the openbb-ecosystem-p1 **G006** landing (the Workspace
control-plane MCP — the workspace-mcp-equivalent). The ecosystem ships two MCP
surfaces: a DATA MCP (financial data — `tdw-mcp`, which also carries the
read-only widget-catalog tools) and a *control-plane* MCP that manipulates the
Workspace itself. This crate is that control plane: an MCP server whose
`tdw.workspace.*` tools mutate a `WorkspaceState` dashboard/layout document that
conforms to the `apps.json` shape and is validated against the `tdw-widgets`
widget catalog.

## Evidence Snapshot

- Manifest: `crates/tdw-workspace-mcp/Cargo.toml`.
- Targets: lib, bin (`tdw-workspace-mcp`, a stdio JSON-RPC server).
- Local deps: `tdw-widgets` (widget catalog + `apps.json` document types) and
  `tdw-endpoint-catalog` (per-route request params schema). Plus `serde` /
  `serde_json` for the wire and the document.
- Optional deps: none.
- Reverse deps: none yet (the control plane is a leaf binary; a Workspace host or
  agent drives it over stdio).
- Features: none (`default` is empty — pure-Rust, no feature gates).
- Tests: `WorkspaceState` unit tests (catalog-validated add rejecting unknown
  widget ids; out-of-bounds / zero-size / overlap rejection on add, move, and
  resize; adjacent non-overlapping placement; navigate-to-unknown rejection;
  delete-active reselection; full CRUD lifecycle; `apps.json` round-trip) and
  server unit tests (initialize negotiation; initialize-before-use guard;
  `tools/list` surface; tool-error vs protocol-error distinction; end-to-end CRUD
  + layout over the JSON-RPC line handler; schema delegation; unknown-tool
  error). A golden integration test (`tests/golden_apps_json.rs`) pins the
  canonical emitted `apps.json` layout (re-bless with `TDW_WORKSPACE_MCP_BLESS=1`).
- Docs/examples: this worksheet plus module-level docs on every public item.

## Control-plane boundary (distinct from the data MCP)

`tdw-mcp` is the DATA surface: it serves market data and the READ-ONLY
widget-catalog tools (`tdw.widgets.list` / `tdw.widgets.describe`). It has no
write/control surface over a Workspace layout. This crate is that missing surface
and is deliberately a *separate* binary so the two MCP servers stay independently
deployable. It reuses, rather than duplicates, the widget catalog: every placed
widget id is validated against `tdw_widgets::catalog_widgets()` — the same catalog
the read-only tools and every widget citation resolve back to — and the layout it
emits serializes through `tdw_widgets::AppConfig` to the exact `apps.json` shape a
Workspace app loads.

The JSON-RPC serve loop, `ToolDescriptor` shape, and envelope mirror the data
MCP's minimal stdio pattern (`initialize` / `ping` / `tools/list` / `tools/call`
over newline-delimited JSON-RPC 2.0), kept self-contained here (plain `std`, no
shared server crate) so the control plane stays a small pure-Rust packet.

## Release Assessment

- `WorkspaceState` is the document model: named dashboards, each a tab carrying a
  grid `layout` of placed widgets, plus an active selection. Every mutation
  enforces an invariant the Workspace frontend would otherwise own: a placed
  widget id must exist in the catalog; a placement must have positive size, fit
  the bounded 40-column grid, and not overlap a placed widget; move/resize
  preserve the complementary dimension and re-validate; navigate targets must
  exist. Domain failures surface as readable `isError` tool results (so the agent
  can recover), while malformed arguments surface as JSON-RPC protocol errors.
- Tool surface (`tdw.workspace.*`): `dashboard.list` / `.create` / `.delete` /
  `.rename`; `widget.add` / `.move` / `.resize` / `.remove`; `widget.schema`
  (delegates to the widget catalog); `layout.get` (the `apps.json`-shaped layout);
  `navigate`.
- Clean-room: built only against the PUBLIC Workspace `apps.json` / `widgets.json`
  contract (the tabs + grid + widget-composition shape already projected by
  `tdw-widgets`). No reference implementation was consulted; no provider source.

## Verdict

Ready with follow-ups. The control-plane core (document model + validation +
stdio MCP serve loop + golden `apps.json` projection) is complete with unit,
server, and golden tests and is pure-Rust/offline. Out of scope for G006 and left
as a later append: persistence of the `WorkspaceState` across process restarts, a
Streamable-HTTP transport alongside stdio, app-level operations (multi-app
management, prompts/`mcp_servers` editing), and richer widget-param validation
(checking a placed widget's bound params against its schema, beyond id validity).
