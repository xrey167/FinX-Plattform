# Secrets and TLS templates

Operator-facing templates for injecting FinX-Plattform secrets (the
`TDW_MCP_HTTP_TOKEN`, provider/LLM keys, Postgres credentials) into systemd and
Kubernetes deployments, plus TLS notes for the Postgres connection and a token
rotation procedure.

This complements:

- [`docs/CONFIGURATION.md`](../CONFIGURATION.md) — the full env-var reference.
- [`docs/release/mcp-remote-deployment.md`](mcp-remote-deployment.md) — TLS /
  reverse-proxy / OAuth in front of the MCP HTTP server (the binary speaks plain
  HTTP; TLS is terminated in front of it).
- [`docs/release/production-auth-oidc.md`](production-auth-oidc.md) — production
  ingress auth (`TDW_OIDC_*`).

None of the `tdw-*` binaries read a `.env` file at runtime (only Docker Compose
does). On a host or in Kubernetes, inject configuration as process environment
variables using the templates below.

## systemd `EnvironmentFile=`

Keep secrets out of the unit file. Put them in a root-owned, `0600` env file and
reference it with `EnvironmentFile=`.

`/etc/tdw/tdw-mcp.env` (mode `0600`, owner `root` or the service user):

```ini
TDW_PROFILE=prod
TDW_MCP_HTTP_TOKEN=REPLACE_WITH_OPENSSL_RAND_HEX_32
TDW_MCP_DAEMON_ADDR=127.0.0.1:7878
```

`/etc/systemd/system/tdw-mcp.service`:

```ini
[Unit]
Description=tdw-mcp Streamable HTTP server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=tdw
Group=tdw
# Secrets live here, not in the unit. 0600, root-owned.
EnvironmentFile=/etc/tdw/tdw-mcp.env
ExecStart=/usr/local/bin/tdw-mcp --streamable-http 127.0.0.1:8788
Restart=on-failure
RestartSec=2
# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

Generate the token and lock down the file before first start:

```bash
install -d -m 0750 -o root -g tdw /etc/tdw
umask 077
printf 'TDW_MCP_HTTP_TOKEN=%s\n' "$(openssl rand -hex 32)" >> /etc/tdw/tdw-mcp.env
chmod 0600 /etc/tdw/tdw-mcp.env
systemctl daemon-reload
systemctl enable --now tdw-mcp.service
```

Bind to `127.0.0.1` and terminate TLS in a reverse proxy in front of it
(`mcp-remote-deployment.md`). Bind to `0.0.0.0` only when the proxy is on a
different host and `TDW_MCP_HTTP_TOKEN` is set.

## Kubernetes Secret + env injection

Create a `Secret` and project it into the pod with `envFrom` / `secretKeyRef`.
Never bake secrets into the image or a `ConfigMap`.

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: tdw-secrets
  namespace: tdw
type: Opaque
stringData:
  # Generate with: openssl rand -hex 32
  TDW_MCP_HTTP_TOKEN: "REPLACE_WITH_OPENSSL_RAND_HEX_32"
  # Use a sslmode=verify-full URL — see "Postgres TLS" below.
  TDW_DAEMON_PG_URL: "postgres://tdw:REPLACE@postgres.tdw.svc:5432/tdw?sslmode=verify-full&sslrootcert=/etc/tdw/tls/ca.crt"
  # Provider / LLM keys as needed:
  POLYGON_API_KEY: "REPLACE"
  ANTHROPIC_API_KEY: "REPLACE"
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: tdw-mcp
  namespace: tdw
spec:
  replicas: 1
  selector:
    matchLabels: { app: tdw-mcp }
  template:
    metadata:
      labels: { app: tdw-mcp }
    spec:
      containers:
        - name: tdw-mcp
          image: finx-plattform/tdw-mcp:local
          args: ["--streamable-http", "0.0.0.0:8788"]
          env:
            - name: TDW_PROFILE
              value: prod
            - name: TDW_MCP_DAEMON_ADDR
              value: tdw-service-daemon:7878
          # Inject every key from the Secret as environment variables.
          envFrom:
            - secretRef:
                name: tdw-secrets
          ports:
            - containerPort: 8788
```

To inject a single key instead of the whole Secret, use `valueFrom`:

```yaml
          env:
            - name: TDW_MCP_HTTP_TOKEN
              valueFrom:
                secretKeyRef:
                  name: tdw-secrets
                  key: TDW_MCP_HTTP_TOKEN
```

Terminate TLS at the Ingress / Gateway in front of the Service.

## `TDW_MCP_HTTP_TOKEN` rotation

The token is a static bearer credential validated by string compare. Rotate it
without dropping requests using a brief dual-token window at the proxy layer:

1. **Generate** a new token: `openssl rand -hex 32`.
2. **Accept both** old and new at the reverse proxy / ingress (configure it to
   pass either bearer value through, or to inject the bearer itself).
3. **Roll** the server's `TDW_MCP_HTTP_TOKEN` to the new value:
   - systemd: edit `/etc/tdw/tdw-mcp.env`, then
     `systemctl restart tdw-mcp.service`.
   - Kubernetes: update the `Secret`, then restart the rollout
     (`kubectl rollout restart deployment/tdw-mcp -n tdw`); env vars are
     re-read at container start.
4. **Update clients** to send the new token.
5. **Stop accepting** the old token at the proxy and scrub it from history/logs.

The server reads `TDW_MCP_HTTP_TOKEN` only at process start, so a restart (not a
hot reload) is required for the new value to take effect.

## Postgres TLS connection string

Use a TLS-verified URL for `TDW_DAEMON_PG_URL` / `TDW_WORKER_PG_URL` /
`DATABASE_URL` in production (the underlying `sqlx` connector honors libpq-style
`sslmode`):

```text
postgres://tdw:PASSWORD@postgres.example:5432/tdw?sslmode=verify-full&sslrootcert=/etc/tdw/tls/ca.crt
```

- `sslmode=verify-full` — encrypt **and** verify the server hostname against its
  certificate (strongest; use this in production).
- `sslmode=verify-ca` — encrypt and verify the CA chain only.
- `sslmode=require` — encrypt but do not verify (weak; avoid).
- `sslrootcert` — path to the CA bundle that signed the server cert (mount it
  into the container, e.g. from a Secret or ConfigMap).

For client-certificate auth, add `sslcert=/etc/tdw/tls/client.crt` and
`sslkey=/etc/tdw/tls/client.key`. Keep the key `0600` and inject it via the same
Secret mechanism as the token.
