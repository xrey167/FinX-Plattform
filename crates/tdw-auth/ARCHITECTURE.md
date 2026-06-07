# tdw-auth architecture

A single-module crate (`src/lib.rs`) implementing fail-closed, role-based query
authorization.

## Module map

| Item | Role |
|------|------|
| `Principal` | `{ subject, roles }` — who is asking. |
| `AuthPolicy` | `{ table, required_role, row_filter }` — what the table demands. |
| `AuthorizationDecision` | `Allow` / `Deny(AuthorizationDenyReason)`. |
| `AuthorizationDenyReason` | `EmptySubject` / `InvalidPolicyTable` / `EmptyRequiredRole` / `InvalidRole` / `MissingRequiredRole`. |
| `authorize(...) -> bool` | Boolean shim over the typed form. |
| `authorize_with_decision(...) -> AuthorizationDecision` | The full check. |
| `is_role_name` / `is_table_path` / `is_table_part` (private) | Identifier guards. |

## Authorization contract

`authorize_with_decision` short-circuits on the first failure, ordering cheap and
safety checks before the role match:

```
subject non-empty
  -> table is a valid schema.table path
  -> required_role non-empty
  -> every principal role and the required role are valid role names
  -> principal holds required_role
  -> Allow
```

`authorize` is simply `authorize_with_decision(principal, policy) == Allow`, for
callers that don't need the reason.

### Why each guard exists

- **`is_table_path`** splits on `.` and requires *exactly two* non-empty segments
  of `[A-Za-z0-9_]` (schema + table). This rejects multi-part or metacharacter
  table strings such as `analytics.gold_daily_returns;drop`, so a policy table
  can be safely interpolated downstream.
- **`is_role_name`** restricts roles to `[A-Za-z0-9_-]+`, blocking control
  characters or separators smuggled into a role (e.g. `analyst\nadmin`).
- The empty-subject / empty-required-role checks make "missing information" a
  denial, not an accidental match.

`row_filter` is carried on the policy for downstream query construction; this
crate validates the principal/role/table relationship and does not itself execute
the filter.

## Relationship to the rest of the platform

`tdw-auth` answers *authorization* (role vs. policy). `tdw-auth-oidc` answers the
upstream *claim/JWKS consistency* question. A request typically passes OIDC claim
validation first, yielding the principal's roles, which `tdw-auth` then checks
against the per-table policy.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Fail closed** — malformed input denies; access requires a positive match.
- **Injection-safe identifiers** for table paths and role names.
- **Pure / deterministic** — no I/O, decision is a function of its inputs.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
