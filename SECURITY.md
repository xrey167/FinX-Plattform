# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 1.x (latest minor) | ✅ security fixes |
| < 1.0 | ❌ |

Releases are tag-driven (see [`docs/release.md`](docs/release.md)); fixes land
on `main` and ship in the next tagged release. There are no long-lived release
branches — upgrade to the latest `v1.x` tag to receive fixes.

## Reporting a vulnerability

**Please do not open a public issue for security reports.**

Use GitHub's private vulnerability reporting: **Security → Report a
vulnerability** on this repository. You should receive an acknowledgement
within **72 hours** and a triage decision (accepted / declined / needs-info)
within **7 days**. Coordinated disclosure is preferred; we'll agree on a
disclosure timeline in the advisory thread (default: when the fix ships in a
tagged release).

If private reporting is unavailable for any reason, contact the maintainer
through the email on the GitHub profile of the repository owner.

## Scope notes for researchers

Areas of particular interest:

- **Daemon and service surfaces** — the daemon binds loopback by default;
  anything that makes a default deployment remotely reachable, bypasses the
  OIDC/token auth (`docs/release/production-auth-oidc.md`), or defeats the
  constant-time token comparison is in scope.
- **MCP server (`tdw-mcp`)** — tool-call paths that could exfiltrate provider
  API keys (read from environment variables at fetch time), reach hosts other
  than the provider's compiled-in base URL, or escape the sandboxed UDF/tool
  execution gating.
- **SQL/query surfaces** — the read-only SQL gate on `RunQuery`-style ops and
  the SQL identifier/denylist validators shared by the Postgres stores.
- **Supply chain** — the release workflow builds binaries and multi-arch
  images with provenance attestation; `cargo audit` runs in CI. Reports about
  the integrity of that pipeline are welcome.

Out of scope: vulnerabilities requiring a non-default configuration explicitly
documented as unsafe (e.g. binding the daemon to a public interface without
auth), and rate-limit/DoS findings against third-party data providers.

## Hardening references

- [`docs/release/secrets-and-tls.md`](docs/release/secrets-and-tls.md)
- [`docs/release/production-auth-oidc.md`](docs/release/production-auth-oidc.md)
- [`docs/quality/`](docs/quality/) — security-audit findings and themes that
  shaped the current validators and bounded-I/O policy.
