# ADR 0013: Release Packaging

Status: Accepted

## Context

G014 requires FinX-Plattform to move from source-only verification to
shippable artifacts. The workspace currently exposes four runnable binaries:
`tdw-service`, `tdw-cli`, `tdw-mcp`, and `tdw-worker`. The release process must
produce host binaries, container images, local Compose smoke coverage, and a
clear versioning contract without changing the clean-room source boundary.

## Decision

Each runnable binary gets its own multi-stage Dockerfile under `docker/`.
Docker builds use `cargo-chef` to keep dependency compilation in a reusable
layer, then copy only the release binary into a `debian:bookworm-slim` runtime
image that runs as a non-root `tdw` user.

`docker-compose.yaml` remains the local stack source of truth. It keeps
storage services in the existing `minimal` and `full` profiles, adds
`tdw-service` and `tdw-worker` to the `full` profile, and adds `tdw-cli` and
`tdw-mcp` to a `tools` profile for one-shot local validation.

GitHub CI builds and scans each image on PRs and pushes. On `main`, CI pushes
the scanned images to GHCR with both `sha-<git-sha>` and `latest` tags.

GitHub Releases are created from `vMAJOR.MINOR.PATCH` tag pushes. The release
workflow builds all four binaries for Linux x86_64, macOS arm64, and Windows
x86_64, uploads compressed archives and SHA-256 checksums, and requests GitHub
build-provenance attestations for the published assets.

The version policy lives in `docs/release.md`. Pre-1.0 releases increment
`MINOR` for user-visible runtime/protocol/storage/provider/release changes and
`PATCH` for compatible fixes.

## Consequences

- Release packaging is reproducible from GitHub Actions and locally inspectable
  through Compose.
- Image vulnerability scanning is part of normal CI instead of a manual release
  step.
- Host binaries and container images are generated from the same source commit,
  but G016 still needs to prove an actual release tag and published images exist
  after G014 lands.
- The service binaries still run the deterministic G009 smoke path; G015/G016
  can expand runtime policy and production-functional coverage without replacing
  the packaging foundation.
