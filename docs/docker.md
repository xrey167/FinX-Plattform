# Docker And WSL2 Notes

The local stack has three Compose profiles:

```powershell
docker compose --profile minimal config
docker compose --profile full config
docker compose --profile tools config
```

`minimal` starts Postgres and ClickHouse. `full` adds Qdrant, Meilisearch,
MinIO, Redis, `tdw-service`, and `tdw-worker`. `tools` adds one-shot
`tdw-cli` and `tdw-mcp` services for local packaged-command checks.

Run the G014 packaged smoke path through Compose:

```powershell
docker compose --profile full run --rm --build tdw-service --smoke AAPL
docker compose --profile full run --rm --build tdw-worker
docker compose --profile tools run --rm --build tdw-cli AAPL
docker compose --profile tools run --rm --build tdw-mcp
```

The binary Dockerfiles live under `docker/` and are also built by CI:

- `docker/tdw-service.Dockerfile`
- `docker/tdw-cli.Dockerfile`
- `docker/tdw-mcp.Dockerfile`
- `docker/tdw-worker.Dockerfile`

On Windows, keep Docker Desktop on the WSL2 backend and leave database storage
on named Docker volumes rather than bind-mounting large database directories
from `C:\`. Named volumes avoid slow host-filesystem metadata paths and keep the
same Compose file usable on Linux CI.

If a bind mount is needed for fixtures, prefer a path inside a WSL2 distro, for
example `/home/<user>/finx-fixtures`, and expose it through Docker Desktop's WSL
integration. Do not put high-write database directories under OneDrive,
Desktop, or Documents.

Current local caveat: Docker Desktop is required for live container smoke tests.
The offline Rust gates and Compose YAML are still runnable without a daemon.

Release packaging details and the SemVer policy are documented in
`docs/release.md`.
