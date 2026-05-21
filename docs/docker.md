# Docker And WSL2 Notes

The local stack has two Compose profiles:

```powershell
docker compose --profile minimal config
docker compose --profile full config
```

`minimal` starts Postgres and ClickHouse. `full` adds Qdrant, Meilisearch,
MinIO, and Redis.

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
