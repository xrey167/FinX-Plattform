# Remote MCP HTTP deployment - TLS, reverse proxy, and OAuth

Operational guide for exposing the `tdw-mcp` Streamable HTTP server beyond
localhost. It closes follow-up #1 in
[`docs/quality/mcp-worker-product-boundaries.md`](../quality/mcp-worker-product-boundaries.md):
the HTTP transport is local-first by design, so remote exposure is a
deployment-level concern (TLS, reverse proxy, OAuth), not an in-binary feature.

## What the binary gives you

`tdw-mcp --streamable-http [bind]` serves the MCP `2025-06-18` protocol over a
single endpoint. Source of truth: `crates/tdw-mcp/src/lib.rs`.

| Property | Value |
|---|---|
| Default bind | `127.0.0.1:8788` (loopback) |
| Endpoint | `POST /mcp` (JSON-RPC 2.0 body) |
| Response | `application/json` or `text/event-stream`, chosen by `Accept` |
| Notifications | `202 Accepted` (fire-and-forget) |
| Origin allowlist | only `http(s)://{localhost,127.0.0.1,::1}`; otherwise `403` |
| Protocol header | `MCP-Protocol-Version` must equal `2025-06-18` when present; otherwise `400` |
| Header cap | 16 KiB (`MAX_HTTP_HEADER_BYTES`); otherwise `431` |
| Body cap | 1 MiB (`MAX_HTTP_BODY_BYTES`); otherwise `413` |
| Bearer auth | required when `TDW_MCP_HTTP_TOKEN` is set (`Authorization: Bearer <token>`) |
| Non-loopback bind | refused (exit code `2`) **unless** `TDW_MCP_HTTP_TOKEN` is set |

Two facts shape the whole topology:

1. **No TLS in the binary.** `tdw-mcp` speaks plain HTTP. TLS must be terminated
   in front of it.
2. **Origin is restricted to loopback.** A browser-origin request from a remote
   site is rejected with `403`. Browser-based MCP clients are therefore **not**
   a supported remote path; non-browser MCP clients (which send no `Origin`
   header) are. Do not try to "fix" this by loosening the allowlist in the
   binary — front it with a proxy instead.

## Recommended topology

```
MCP client ──TLS──> reverse proxy (TLS + OAuth) ──plain HTTP, loopback──> tdw-mcp --streamable-http 127.0.0.1:8788
```

- The proxy owns the public certificate, the OAuth/OIDC handshake, and request
  logging.
- `tdw-mcp` stays bound to `127.0.0.1` on the same host (or a private network
  namespace the proxy can reach). It never faces the public interface.
- `TDW_MCP_HTTP_TOKEN` is set to a high-entropy secret and injected by the proxy
  on every upstream request as a bearer token. This is defense-in-depth: even if
  something reaches the loopback port directly, it must present the token.

If you must bind `tdw-mcp` itself to a non-loopback address (for example in a
sidecar where the proxy is on another host in a trusted subnet), the binary
forces `TDW_MCP_HTTP_TOKEN` to be set first. Treat that as the exception, not
the default.

## Step 1 - run the server on loopback with a token

```powershell
$env:TDW_MCP_HTTP_TOKEN = (python -c "import secrets;print(secrets.token_urlsafe(32))")
tdw-mcp --streamable-http 127.0.0.1:8788
```

Validate locally before fronting it:

```powershell
tdw-mcp --streamable-http-smoke   # initialize + an SSE progress tool call, no listener
```

## Step 2 - terminate TLS and proxy to loopback

### Caddy (automatic TLS)

```caddyfile
mcp.example.com {
    @mcp path /mcp
    handle @mcp {
        # OAuth handled by forward_auth or an upstream identity proxy; see Step 3.
        reverse_proxy 127.0.0.1:8788 {
            header_up Authorization "Bearer {env.TDW_MCP_HTTP_TOKEN}"
            # Drop any client-supplied Origin so the loopback allowlist is satisfied.
            header_up -Origin
        }
    }
    respond 404
}
```

### nginx

```nginx
server {
    listen 443 ssl;
    server_name mcp.example.com;

    ssl_certificate     /etc/ssl/mcp.example.com.crt;
    ssl_certificate_key /etc/ssl/mcp.example.com.key;

    location = /mcp {
        # auth_request points at your OAuth introspection endpoint (Step 3).
        auth_request /_oauth_introspect;

        proxy_pass http://127.0.0.1:8788/mcp;
        proxy_set_header Authorization "Bearer ${TDW_MCP_HTTP_TOKEN}";
        proxy_set_header Origin "";                 # satisfy loopback Origin check
        proxy_set_header MCP-Protocol-Version "2025-06-18";
        proxy_read_timeout 3600s;                   # SSE streams are long-lived
        proxy_buffering off;                        # stream text/event-stream promptly
    }
}
```

Key proxy requirements regardless of product:

- **Terminate TLS** at the proxy; upstream stays plain HTTP on loopback.
- **Disable response buffering** on `/mcp` so `text/event-stream` progress
  notifications flush to the client immediately.
- **Raise the read timeout** well above the default 60s so SSE tool calls are
  not cut off.
- **Inject the bearer token** upstream from a secret store, not a static config
  file in the image.
- **Normalize `Origin`** (drop it or set it empty) because the binary only
  accepts loopback origins.
- Keep client bodies under **1 MiB** and headers under **16 KiB**, or return a
  proxy-level `413`/`431` with a clear message before the request reaches the
  server.

## Step 3 - OAuth / OIDC at the edge

`tdw-mcp` does not implement OAuth; it only checks a static bearer token. Put a
real identity layer in the proxy tier:

- **OAuth2 Resource Server pattern.** Front the proxy with an identity-aware
  proxy (e.g. `oauth2-proxy`, Pomerium, or your cloud load balancer's IAP) that
  validates the caller's OIDC token and then forwards to nginx/Caddy. The MCP
  client obtains an access token from your IdP and presents it on `/mcp`.
- The edge validates the user/client token (signature, audience, expiry,
  scope), and **only then** swaps in the internal `TDW_MCP_HTTP_TOKEN` for the
  upstream hop. The internal token is never exposed to clients.
- Scope mapping: gate `/mcp` on a dedicated scope (for example
  `mcp:invoke`) so MCP access can be granted independently of other APIs.

This keeps the trust boundary clean: clients authenticate with short-lived OIDC
tokens at the edge; the server authenticates the edge with a long-lived shared
secret on loopback.

## Hardening checklist

- [ ] `tdw-mcp` bound to `127.0.0.1` (or a private namespace), never the public
      interface.
- [ ] `TDW_MCP_HTTP_TOKEN` set from a secret manager, rotated on a schedule.
- [ ] TLS terminated at the proxy with a current certificate; HTTP redirected
      to HTTPS.
- [ ] OAuth/OIDC validated at the edge before the proxy forwards to `/mcp`.
- [ ] Response buffering off and read timeout raised for SSE on `/mcp`.
- [ ] Proxy enforces the 1 MiB body / 16 KiB header caps and rate-limits `/mcp`.
- [ ] Access logs on the proxy capture client identity (not the shared token).
- [ ] The server process runs unprivileged (see worker/service container notes).

## What this does NOT cover

- Multi-tenant authorization inside MCP tool calls. The token/OAuth gate is
  coarse (access to the endpoint); per-tool or per-tenant policy is a separate
  concern.
- Horizontal scaling / sticky sessions. The stdio and HTTP servers are stateful
  per connection; running multiple replicas behind the proxy needs session
  affinity, which is not yet specified here.
- Certificate issuance and secret-manager wiring, which are environment
  specific.

## See also

- [`docs/quality/mcp-worker-product-boundaries.md`](../quality/mcp-worker-product-boundaries.md)
  - what shipped in-binary vs. what is a deployment concern.
- [`docs/release/worker-deployment.md`](worker-deployment.md) - the companion
  `PgWorkerQueue` operational guide.
- `crates/tdw-mcp/src/lib.rs` - authoritative bind, auth, Origin, protocol, and
  size-limit logic.
