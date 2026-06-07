# Production OIDC auth (`TDW_OIDC_*`)

Single reference for configuring the `tdw-service` daemon's ingress auth in a
`prod`/`production` profile. The daemon is **fail-closed by default**: with no
auth configured it attaches no policy and every dispatch returns `Failed`.
Set the variables below to attach an auth-backed policy.

> **Verification model.** The `TDW_OIDC_*` policy builder performs the
> **structural** claim/JWKS consistency pre-filter described below (issuer,
> audience, subject, `kid` ∈ JWKS, allowed algorithm, role-name shape).
> **Cryptographic** JWT verification — RS256/ES256 signature validation against
> the supplied verifying keys plus `exp`/`nbf`/`iat` (60s clock-skew leeway),
> issuer, and audience enforcement — is implemented in `tdw-auth-oidc`'s
> `verify_jwt` / `verify_jwt_strict`, which reject the `none` pseudo-algorithm
> and HMAC tokens (alg-confusion / `alg:none` defence) and fail closed on any
> error. Remote JWKS fetch/refresh remains out of scope: verifying keys are
> supplied from the configured JWKS rather than fetched at runtime.

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
  is the default fail-closed posture and is logged as a generic message; the
  daemon **starts** (every dispatch returns `Failed` until a policy is wired).
- **Some required vars present, others missing** → the daemon **refuses to
  start**. A partial misconfiguration is an operator error, not a silent
  default: startup returns an error whose diagnostic lists **every** missing
  variable. Set the listed variables, or unset all of them to run fail-closed.
- **All required present but parse/validation fails** (malformed `kid:alg` pair,
  duplicate `kid`, unknown `kid`, unsupported algorithm, audience/issuer/subject
  mismatch, invalid role) → the daemon **refuses to start** and the error names
  the specific cause.
- **All valid** → an auth-backed policy is attached and dispatches resolve.

## Boot diagnostics

The daemon prints/returns one of the following at startup (`{profile}` is the
effective profile):

- Policy attached (stderr, then continues):
  `tdw-backend: daemon starting in '{profile}' profile with a policy attached; dispatches will resolve`
- Partial/invalid prod config (**startup error — the daemon does not start**):
  `refusing to start daemon in '{profile}' profile: partial OIDC config: missing TDW_OIDC_JWKS, TDW_OIDC_SUBJECT, TDW_OIDC_KID; set the listed TDW_OIDC_* variables (or unset all of them to run fail-closed)`
  or e.g. `... refusing to start ...: invalid claims: UnknownKeyId; ...`
- Fully-unset (generic fail-closed, stderr, then continues):
  `tdw-backend: daemon starting in '{profile}' profile with no policy attached; dispatches will return Failed until an auth-backed policy is wired (configure TDW_OIDC_*)`

The specific-cause variants are produced by the typed `OidcPolicyError`
(`MissingEnvVars`, `MalformedJwksPair`, `DuplicateKid`, `InvalidClaims`) in
`tdw-service-api`; `run_daemon` turns any of them into a hard startup error.

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

To verify the startup contract, start the daemon with a *partial* set (e.g. drop
`TDW_OIDC_JWKS`) and confirm the daemon **refuses to start** with an error
listing every missing variable; with all valid, confirm "policy attached"; with
all unset, confirm the daemon starts and logs the generic fail-closed message.

## Evidence

`tdw-service-api` carries pure unit tests for each `OidcPolicyError` variant and
the all-unset / partial / all-valid wrapper semantics (driven by an injected
lookup, no process-env mutation), plus end-to-end tests proving a prod-built
policy resolves a `RunQuery` dispatch to `Completed` and rejects tampered
ingress claims. See `crates/tdw-service-api/src/app_state.rs` tests.
