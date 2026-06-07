# tdw-auth-oidc

OIDC claim / JWKS **structural** validation for the daemon ingress boundary.

Given a set of already-decoded JWT claims and a local JWKS key set, this crate
answers a single question: *are these claims internally consistent with the
expected issuer/audience and with a key in the JWKS?* It is the claim-consistency
half of the daemon's OIDC policy.

> **Scope — read this first.** As of the current source, this crate performs
> **structural validation only**. It does **not** verify JWT signatures, decode
> raw tokens, fetch/refresh a remote JWKS, or check token expiry. The crate's
> own module docs are explicit: *"It does not verify JWT signatures, parse/decode
> tokens, fetch or refresh a remote JWKS, or check token expiry — those are
> tracked follow-ups for the auth-service adapter."* If you need cryptographic
> JWT verification, that lives (or will live) in the auth-service adapter, not
> here. This README documents the code as it actually is; do not assume a
> cryptographic verifier that the source does not contain.

## What it provides

- `JwtClaims` — decoded claims: `sub`, `iss`, `aud`, `kid`, `roles`.
- `JwksKey` — one JWKS entry: `kid`, `alg`.
- `validate_claims_strict(...)` — full check returning a rich
  `ClaimValidationError` and accepting an explicit allowed-algorithm list.
- `validate_claims(...)` — a `bool` shim over `validate_claims_strict` using
  `DEFAULT_ALLOWED_ALGORITHMS` (`["RS256", "ES256"]`).
- `ClaimValidationError` — the typed reason a claim set was rejected.

## Feature flags

None. The crate's only dependency is `serde`.

## Quickstart

```rust
use tdw_auth_oidc::{validate_claims, JwksKey, JwtClaims};

let jwks = [JwksKey { kid: "k1".into(), alg: "RS256".into() }];
let claims = JwtClaims {
    sub: "alice".into(),
    iss: "https://issuer.example".into(),
    aud: "tdw".into(),
    kid: "k1".into(),
    roles: vec!["analyst".into()],
};

assert!(validate_claims(&claims, &jwks, "https://issuer.example", "tdw"));
// An empty JWKS fails closed:
assert!(!validate_claims(&claims, &[], "https://issuer.example", "tdw"));
```

See [`examples/basic.rs`](examples/basic.rs) (mirrors the crate's own tests):

```sh
cargo run -p tdw-auth-oidc --example tdw_auth_oidc_basic
```

## Validation rules (in order)

`validate_claims_strict` rejects, with the matching `ClaimValidationError`:

1. empty/whitespace `sub` → `EmptySubject`
2. `iss` mismatch or empty expected issuer → `IssuerMismatch`
3. `aud` mismatch or empty expected audience → `AudienceMismatch`
4. empty `kid` → `EmptyKeyId`
5. any role that is not `[A-Za-z0-9_-]+` → `InvalidRole`
6. `kid` not present in the JWKS → `UnknownKeyId`
7. the matched key's `alg` not in the allowed list → `UnsupportedAlgorithm`

## Invariants

- `#![forbid(unsafe_code)]`.
- **Fail closed.** Anything not provably consistent is rejected: an empty JWKS,
  an empty expected issuer/audience, an unknown `kid`, or an algorithm outside
  the allow-list all deny.
- **Algorithm allow-list, not deny-list.** Only explicitly allowed algorithms
  pass (`alg: "none"` is rejected because it is not in the default list).
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
