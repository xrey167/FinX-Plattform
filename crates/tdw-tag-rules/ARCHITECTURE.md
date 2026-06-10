# tdw-tag-rules — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `RulePredicate` | `SqlContains`, `JsonPathEquals`, `LabelContains`. |
| `TagRule` | `rule_id`, `tag_id`, `predicate`. |
| `RuleEngine` | Versioned rule set + apply loop. |
| `RuleError` | `UnsafeSql`, `InvalidRule`, `Tag`, `Recursion`. |
| `hot_reload` / `apply` / `version` | Engine API. |
| `validate_rule` / `is_identifier` / `is_tag_id` (private) | Load-time checks. |

## Key types and traits

- `RulePredicate` and `TagRule` derive `Clone, Debug, PartialEq, Eq, Serialize,
  Deserialize`, so rule sets can be loaded from config.
- `RuleEngine` derives `Clone, Debug, Default`; `rules` and `version` are private.
- `RuleError` uses `thiserror`. `Tag(String)` wraps a `tdw_tags::TagError` message
  raised when an assignment fails (e.g. the target tag is not defined).

## Predicate / hot-reload / apply model

```
hot_reload(rules):
    validate every rule (fail-fast) — bad SQL / json-path / empty fields rejected
    swap rules atomically; version += 1

apply(entity_id, label, &mut store):
    for each rule:
        matched = match predicate {
            SqlContains { needle, .. }  => label.contains(needle),
            JsonPathEquals { value, .. } => label == value,
            LabelContains { label: l }   => label.contains(l),
        }
        if matched:
            build TagAssignment { entity_id, tag_id, assigned_at: now, provenance: "rule:<rule_id>@v<version>" }
            store.assign(assignment)?      // may fail -> RuleError::Tag
            collect assignment
    ▶ Vec<TagAssignment>
```

`hot_reload` is all-or-nothing: if any rule fails validation the existing rule set
is left untouched and the version does not advance. `apply` writes through to the
`TagStore`, so the store's own validation (defined tag, valid assignment) is the
second line of defence — a rule referencing an undefined tag surfaces as
`RuleError::Tag`.

## Invariants

- **`rule_id` grammar**: non-empty ASCII alphanumeric plus `_`, `-` (e.g.
  `../bad` → `InvalidRule("rule_id")`).
- **`tag_id` grammar**: non-empty, must contain `:`, ASCII alphanumeric plus `:`,
  `_`, `-` (matches `tdw-tags`).
- **SQL safety** (`SqlContains`): the `sql` string may not contain `;`, `--`, or a
  ` drop ` token, and `needle` must be non-empty → otherwise `UnsafeSql` /
  `InvalidRule`.
- **JSON path** (`JsonPathEquals`): `path` must start with `$.`, contain no `..`
  and no control chars; `value` non-empty.
- **Label** (`LabelContains`): non-empty, control-char-free.
- Every applied assignment carries `provenance = "rule:<rule_id>@v<version>"`, so produced
  tags are auditable back to the rule that created them.
- Pure and deterministic; `assigned_at` is the caller-injected `now` date, so the
  apply path takes no clock itself. No I/O, no global state.
