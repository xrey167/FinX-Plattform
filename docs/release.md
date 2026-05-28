# Release Packaging

Scope: G014 release packaging for `tdw-service`, `tdw-cli`, `tdw-mcp`,
and `tdw-worker`.

## Version Policy

FinX-Plattform uses SemVer tags in the form `vMAJOR.MINOR.PATCH`.

Until the platform reaches `v1.0.0`:

- Increment `MINOR` for user-visible runtime, protocol, storage, provider,
  or release-packaging changes.
- Increment `PATCH` for compatible fixes, docs, CI-only changes, and
  packaging repairs that do not change runtime behavior.
- Keep breaking protocol or persistence changes behind migration notes in the
  release notes, even while the major version is `0`.

The GitHub release workflow only packages tag pushes matching
`vMAJOR.MINOR.PATCH`.

## Release Artifacts

`.github/workflows/release.yml` builds these binaries:

- `tdw-service`
- `tdw-cli`
- `tdw-mcp`
- `tdw-worker`

Each binary is built for:

- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

The workflow uploads compressed archives plus `.sha256` checksum files to the
GitHub Release. It also requests GitHub build-provenance attestations for the
published archive and checksum files.

## Container Images

CI builds and scans a container image for every packaged binary:

- `docker/tdw-service.Dockerfile`
- `docker/tdw-cli.Dockerfile`
- `docker/tdw-mcp.Dockerfile`
- `docker/tdw-worker.Dockerfile`

On `main` pushes, the scanned images are pushed to GHCR as:

```text
ghcr.io/<owner>/finx-plattform-tdw-service:sha-<git-sha>
ghcr.io/<owner>/finx-plattform-tdw-service:latest
ghcr.io/<owner>/finx-plattform-tdw-cli:sha-<git-sha>
ghcr.io/<owner>/finx-plattform-tdw-cli:latest
ghcr.io/<owner>/finx-plattform-tdw-mcp:sha-<git-sha>
ghcr.io/<owner>/finx-plattform-tdw-mcp:latest
ghcr.io/<owner>/finx-plattform-tdw-worker:sha-<git-sha>
ghcr.io/<owner>/finx-plattform-tdw-worker:latest
```

The scan gate fails on unfixed high or critical vulnerabilities reported by
Trivy for the built image.

## Local Compose Smoke

Validate the Compose model without a Docker daemon:

```powershell
docker compose --profile minimal config
docker compose --profile full config
docker compose --profile tools config
```

Run the packaged smoke path through Compose:

```powershell
docker compose --profile full run --rm --build tdw-service --smoke AAPL
docker compose --profile full run --rm --build tdw-worker
docker compose --profile tools run --rm --build tdw-cli AAPL
docker compose --profile tools run --rm --build tdw-mcp
```

`tdw-service` and `tdw-cli` execute the G009 deterministic smoke path. The
`full` profile also declares the production storage services so the same
composition can be extended by G015/G016 without changing the service graph.

## Release Cut Checklist

1. Ensure `main` is green on CI and CodeQL.
2. Choose the next SemVer tag using the policy above.
3. Create and push the tag:

   ```powershell
   git tag v0.MINOR.PATCH
   git push origin v0.MINOR.PATCH
   ```

4. Wait for the Release workflow to publish all 12 archives, checksum files,
   and attestations.
5. Confirm the CI image job pushed fresh GHCR `sha-<git-sha>` images from the
   same commit.

## Remaining Follow-up

This page defines the G014 release surface. G016 remains responsible for a
final aggregate proof that a release tag exists, images were published from
`main`, and the full production-functional gate is clear.
