# Production OIDC auth (`TDW_OIDC_*`)

Single reference for configuring the `tdw-service` daemon's ingress auth in a
`prod`/`production` profile. The daemon is **fail-closed by default**: with no
auth configured it attaches no policy and every dispatch returns `Failed`.
Set the variables below to attach an auth-backed policy.

> **Scope caveat — structural, not cryptographic.** This validates claim/JWKS
> **consistency** (issuer, audience, subject, `kid` ∈ JWKS, allowed algorithm,
> role-name shape). It does **not** verify JWT cryptographic signatures, fetch a
> remote JWKS, or check token expiry. Full signature verification + JWKS
> transport/refresh is a tracked follow-up in `tdw-auth-oidc`.

## When it applies

The `TDW_OIDC_*` variables are read **only** when the effective profile is
`prod` or `production` (set via `TDW_PROFILE`). Non-prod profiles (`default`,
`docker`, `service`, …) synthesize a deterministic local-default policy instead
and ignore these variables.

## The six variables

| Variable           | Required | Meaning                                                                 |
|--------------------|----------|-------------------------------------------------------------------------|
| `TDW_OIDC_ISSUER`  | yes      | Expected token issuer (`iss`); must match the principal's claim.        |
| `TDW_OIDC_AUDIENCE`| yes      | Expected audience (`aud`); must match the principal's claim.            |
| `TDW_OIDC_JWKS`    | yes      | Comma-separated `kid:alg` pairs, e.g. `key1:RS256,key2:ES256`.          |
| `TDW_OIDC_SUBJECT` | yes      | Principal subject (`sub`); must be non-empty.                           |
| `TDW_OIDC_KID`     | yes      | The principal's active key id; must be one of the `TDW_OIDC_JWKS` kids. |
| `TDW_OIDC_ROLES`   | no       | Comma-separated roles, e.g. `analyst,udf_runner`; empty/unset → none.   |

Allowed algorithms are `RS256` and `ES256`. Values are trimmed; a blank value
counts as unset.

## Fail-closed semantics

- **All five required vars unset** → no policy attached, *intentionally*. This
  is the default fail-closed posture and is logged as a generic message.
- **Some required vars present, others missing** → fail closed **and** the boot
  log names the first missing variable (a partial misconfiguration is treated as
  an operator error, not a silent default).
- **All required present but parse/validation fails** (malformed `kid:alg` pair,
  duplicate `kid`, unknown `kid`, unsupported algorithm, audience/issuer/subject
  mismatch, invalid role) → fail closed **and** the boot log names the specific
  cause.
- **All valid** → an auth-backed policy is attached and dispatches resolve.

## Boot diagnostics

The daemon prints one of the following to stderr at startup (`{profile}` is the
effective profile):

- Policy attached:
  `tdw-service: daemon starting in '{profile}' profile with a policy attached; dispatches will resolve`
- Partial/invalid prod config (names the cause):
  `tdw-service: daemon starting in '{profile}' profile with no policy attached: TDW_OIDC_JWKS missing; configure TDW_OIDC_* correctly so dispatches resolve`
  or e.g. `... no policy attached: invalid claims: UnknownKeyId; ...`
- Fully-unset (generic fail-closed):
  `tdw-service: daemon starting in '{profile}' profile with no policy attached; dispatches will return Failed until an auth-backed policy is wired (configure TDW_OIDC_*)`

The specific-cause variants are produced by the typed `OidcPolicyError`
(`MissingEnvVar`, `MalformedJwksPair`, `DuplicateKid`, `InvalidClaims`) in
`tdw-service-api`.

## Worked example

```powershell
$env:TDW_PROFILE      = "prod"
$env:TDW_OIDC_ISSUER  = "https://issuer.example"
$env:TDW_OIDC_AUDIENCE= "tdw-daemon"
$env:TDW_OIDC_JWKS    = "key1:RS256,key2:ES256"
$env:TDW_OIDC_SUBJECT = "svc:prod"
$env:TDW_OIDC_KID     = "key1"
$env:TDW_OIDC_ROLES   = "analyst,udf_runner"
```

To verify the fail-closed diagnostics, start the daemon with a *partial* set
(e.g. drop `TDW_OIDC_JWKS`) and confirm the boot log names the missing variable;
with all valid, confirm "policy attached"; with all unset, confirm the generic
fail-closed message.

## Evidence

`tdw-service-api` carries pure unit tests for each `OidcPolicyError` variant and
the all-unset / partial / all-valid wrapper semantics (driven by an injected
lookup, no process-env mutation), plus end-to-end tests proving a prod-built
policy resolves a `RunQuery` dispatch to `Completed` and rejects tampered
ingress claims. See `crates/tdw-service-api/src/app_state.rs` tests.
