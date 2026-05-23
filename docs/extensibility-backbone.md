# Extensibility Backbone

G004 adds the first non-plugin extensibility path:

- `tdw-tools` owns `ToolDefinition`, `ToolRegistry`, `ToolRouter`, and
  `ToolOrchestrator`. The orchestrator applies `tdw-hooks::PermissionRules`
  before invoking a registered tool and can defer approval through a protocol
  permission ID.
- `tdw-sandbox` owns `SandboxRuntime`, `UdfRequest`, and `UdfResponse`. The
  initial `LocalUdfSandbox` delegates to the existing `tdw-udf` contract and
  preserves denied network/filesystem capabilities.
- `tdw-mcp` remains the initial inward MCP server binary. Its catalog is exposed
  through `tdw-service-api::mcp_extensibility_tools` while a future split into
  client/server crates is deferred.
- `tdw-acp` defines the outward request/response boundary using `tdw-protocol`
  `Op` and `EventMsg` types.

`tdw-service-api::extensibility_sample` is the integration proof that the tool
orchestrator, UDF sandbox, MCP tool catalog, and ACP server info are available
from the service surface.
