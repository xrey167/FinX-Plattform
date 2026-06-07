# tdw-auth-oidc architecture

A single-module crate (`src/lib.rs`) implementing pure, allocation-light
structural validation of OIDC claims against a local JWKS.

## Module map

| Item | Role |
|------|------|
| `DEFAULT_ALLOWED_ALGORITHMS: [&str; 2]` | `["RS256", "ES256"]` — the default signing-algorithm allow-list. |
| `JwksKey` | One JWKS entry: `kid`, `alg`. |
| `JwtClaims` | Decoded claims: `sub`, `iss`, `aud`, `kid`, `roles`. |
| `ClaimValidationError` | Enum of rejection reasons. |
| `validate_claims_strict(...)` | The full check; returns `Result<(), ClaimValidationError>`. |
| `validate_claims(...)` | `bool` convenience shim using the default algorithm list. |
| `is_role_name` (private) | Role-name charset guard (`[A-Za-z0-9_-]+`). |

## Design: structural, fail-closed

The crate intentionally draws a hard line at *cryptography*. It assumes some
upstream component has already base64-decoded the JWT into a `JwtClaims` value;
its job is to confirm that those claims are coherent with the configured trust
parameters before the daemon acts on them.

### The fail-closed contract

Every check is written so that *absence of proof is denial*:

- An **empty JWKS** can never validate any claim set (no `kid` can match).
- An **empty expected issuer or audience** is treated as a configuration that
  matches nothing — `IssuerMismatch` / `AudienceMismatch` — rather than as a
  wildcard. This prevents a misconfigured empty string from accidentally
  accepting all tokens.
- The algorithm check is an **allow-list**: only algorithms explicitly passed in
  (or `DEFAULT_ALLOWED_ALGORITHMS` via the shim) are accepted, so `alg: "none"`
  and any unexpected algorithm are rejected by default.
- Role names are restricted to `[A-Za-z0-9_-]+`, blocking injection of control
  characters or separators (e.g. a newline-smuggled `analyst\nadmin`).

### Validation order

`validate_claims_strict` short-circuits on the first failure, checking cheap
field constraints before the JWKS lookup:

```
sub non-empty
  -> iss == issuer (and issuer non-empty)
  -> aud == audience (and audience non-empty)
  -> kid non-empty
  -> every role is a valid role name
  -> kid found in JWKS
  -> matched key's alg in allowed list
  -> Ok(())
```

`validate_claims` is exactly
`validate_claims_strict(.., &DEFAULT_ALLOWED_ALGORITHMS).is_ok()` — a boolean
front door for callers that do not need the typed reason.

## Relationship to the rest of the platform

The production `TDW_OIDC_*` policy builder in `tdw-service-api` calls into this
crate for the claim/JWKS-consistency portion of its decision. The cryptographic
steps this crate explicitly excludes (signature verification, token decoding,
remote JWKS fetch/refresh, expiry) are the responsibility of the auth-service
adapter and are tracked follow-ups; see `docs/release/production-auth-oidc.md`.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy lints).
- **Fail closed**, **allow-list algorithms**, **no implicit wildcards** — see
  above.
- **Clean-room.** No vendor-derived code or branding; the validation is written
  from the OIDC field semantics directly.
- **Pure / no I/O.** No network, no clock, no filesystem — deterministic given
  its inputs, which is what makes it cheap to test exhaustively.
