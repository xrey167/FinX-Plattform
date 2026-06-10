# OIDC with a real IdP: mapping Keycloak / Auth0 / Entra ID onto `TDW_OIDC_*`

Companion to [production-auth-oidc](production-auth-oidc.md), which is the
authoritative contract (the six variables, fail-closed semantics, boot
diagnostics). This page answers the operator question that doc leaves open:
*"where do these six values come from in my identity provider?"*

> **Key design point first:** FinX does **not** fetch JWKS at runtime. You
> supply the expected `kid:alg` pairs (and `tdw-auth-oidc` verifies signatures
> against the configured verifying keys). That means **IdP key rotation is a
> config change on your side** — when your IdP rotates its signing key, update
> `TDW_OIDC_JWKS`/`TDW_OIDC_KID` and restart. Pin rotation reminders to your
> IdP's key-rollover policy.

## What each variable maps to

| `TDW_OIDC_*` | OIDC concept | Where to find it |
|---|---|---|
| `ISSUER` | `iss` claim | The IdP's issuer URL — see per-IdP table below |
| `AUDIENCE` | `aud` claim | The identifier you assign the FinX API/client in the IdP |
| `JWKS` | allowed `kid:alg` set | The `kid` + `alg` of the IdP's **current signing key(s)**, from its JWKS endpoint |
| `SUBJECT` | `sub` claim | The service principal's subject as your IdP emits it |
| `KID` | active key id | The `kid` your principal's tokens are currently signed with (∈ `JWKS`) |
| `ROLES` | role claims | The role names you grant the principal (e.g. `analyst,udf_runner`) |

Discover the live values from any compliant IdP:

```bash
# issuer + jwks_uri:
curl -s https://<idp-issuer>/.well-known/openid-configuration | jq '{issuer, jwks_uri}'
# current signing keys (take "kid" and "alg" — only RS256/ES256 are accepted):
curl -s <jwks_uri> | jq '[.keys[] | {kid, alg, use}]'
# a sample token's claims (paste an actual service token):
cut -d. -f2 <<<"$TOKEN" | base64 -d 2>/dev/null | jq '{iss, aud, sub}'
```

## Per-IdP quick tables

### Keycloak (self-hosted reference)

1. Create a realm (e.g. `finx`), then a **confidential client** with the
   *service accounts* flow enabled (client-credentials).
2. Values:

| Variable | Keycloak value |
|---|---|
| `ISSUER` | `https://<keycloak-host>/realms/finx` |
| `AUDIENCE` | your client ID (add an *audience mapper* if tokens carry a different `aud`) |
| `JWKS` / `KID` | from `https://<keycloak-host>/realms/finx/protocol/openid-connect/certs` — typically one RS256 key |
| `SUBJECT` | the service-account user's ID (`service-account-<client>` → its `sub`; decode one token to confirm) |
| `ROLES` | realm/client roles you assign the service account |

### Auth0

| Variable | Auth0 value |
|---|---|
| `ISSUER` | `https://<tenant>.<region>.auth0.com/` (trailing slash — match the token's `iss` exactly) |
| `AUDIENCE` | the **API identifier** you create under *Applications → APIs* |
| `JWKS` / `KID` | `https://<tenant>.../.well-known/jwks.json` — RS256 |
| `SUBJECT` | `<client_id>@clients` for client-credentials tokens |
| `ROLES` | via a custom claim/Action; FinX reads the role names you configure |

### Microsoft Entra ID (Azure AD)

| Variable | Entra value |
|---|---|
| `ISSUER` | `https://login.microsoftonline.com/<tenant-id>/v2.0` |
| `AUDIENCE` | the app registration's *Application ID URI* (or client ID, matching the token's `aud`) |
| `JWKS` / `KID` | `https://login.microsoftonline.com/<tenant-id>/discovery/v2.0/keys` — RS256; Entra rotates keys frequently, plan for it |
| `SUBJECT` | the service principal's `sub`/`oid` as emitted (decode one token) |
| `ROLES` | app roles assigned to the principal (`roles` claim names) |

## Wire it and verify

```bash
export TDW_PROFILE=prod
export TDW_OIDC_ISSUER="https://<as-above>"
export TDW_OIDC_AUDIENCE="<as-above>"
export TDW_OIDC_JWKS="<kid>:RS256"
export TDW_OIDC_SUBJECT="<principal-sub>"
export TDW_OIDC_KID="<kid>"
export TDW_OIDC_ROLES="analyst"
```

Start the daemon and check the boot diagnostic (the contract doc lists all
variants):

- success → `... daemon starting in 'prod' profile with a policy attached; dispatches will resolve`
- any partial/invalid config → the daemon **refuses to start** and names the
  exact problem (`MissingEnvVars`, `MalformedJwksPair`, `DuplicateKid`,
  `InvalidClaims`).

Negative drill worth doing once: flip `TDW_OIDC_KID` to a bogus value and
confirm the daemon refuses to start with `InvalidClaims: UnknownKeyId` — that
is the fail-closed posture protecting you.

## Rotation runbook (because JWKS is static)

1. IdP publishes a new signing key (old one still valid): add the new pair to
   `TDW_OIDC_JWKS` (`old:RS256,new:RS256`) and restart — tokens under either
   key now pass the structural filter.
2. Principal's tokens switch to the new key: update `TDW_OIDC_KID`, restart.
3. IdP retires the old key: remove it from `TDW_OIDC_JWKS`, restart.

Each step is a config-only change; a mistake fails closed at startup rather
than letting unverifiable tokens through.
