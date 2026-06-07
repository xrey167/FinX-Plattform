# tdw-auth

Role-based authorization for query paths: does a principal hold the role a table
policy requires?

`tdw-auth` is the authorization half of the access boundary (the OIDC claim half
lives in `tdw-auth-oidc`). Given a `Principal` (subject + roles) and an
`AuthPolicy` (a table, the required role, an optional row filter), it returns a
typed allow/deny decision — fail-closed, with the table path and role names
validated against injection.

## What it provides

- `Principal` — `{ subject, roles }`.
- `AuthPolicy` — `{ table, required_role, row_filter }`.
- `authorize(principal, policy) -> bool` — the boolean front door.
- `authorize_with_decision(principal, policy) -> AuthorizationDecision` — the
  typed form, carrying the deny reason.
- `AuthorizationDecision` (`Allow` / `Deny(reason)`) and `AuthorizationDenyReason`.

## Feature flags

None. Depends only on `serde`.

## Quickstart

```rust
use tdw_auth::{authorize, AuthPolicy, Principal};

let policy = AuthPolicy {
    table: "analytics.gold_daily_returns".to_string(),
    required_role: "analyst".to_string(),
    row_filter: Some("tenant_id = current_tenant()".to_string()),
};

assert!(authorize(
    &Principal { subject: "alice".to_string(), roles: vec!["analyst".to_string()] },
    &policy,
));
assert!(!authorize(
    &Principal { subject: "bob".to_string(), roles: vec!["guest".to_string()] },
    &policy,
));
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-auth --example tdw_auth_basic
```

## Decision rules (in order)

`authorize_with_decision` denies with the matching reason:

1. empty/whitespace `subject` → `EmptySubject`
2. `table` not a valid `schema.table` path (two `[A-Za-z0-9_]` segments) →
   `InvalidPolicyTable`
3. empty `required_role` → `EmptyRequiredRole`
4. any principal role or the required role not `[A-Za-z0-9_-]+` → `InvalidRole`
5. the principal does not hold `required_role` → `MissingRequiredRole`

Otherwise → `Allow`. `authorize` is `authorize_with_decision(..) == Allow`.

## Invariants

- `#![forbid(unsafe_code)]`.
- **Fail closed.** Any malformed input (empty subject, unsafe table, bad role)
  denies; access is granted only on a positive role match.
- **Injection-safe identifiers.** The table path is restricted to two
  underscore/alphanumeric segments (rejecting e.g. `analytics.x;drop`), and role
  names to `[A-Za-z0-9_-]+`.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
