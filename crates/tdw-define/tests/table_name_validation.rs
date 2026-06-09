//! Integration tests exercising `DefineEvent::validate` table-name rules via the
//! public API. These drive the private `is_table_name` helper through every
//! reachable branch (valid `schema.table`, no separator, empty parts, extra
//! separators, and illegal characters) without depending on crate internals.

use tdw_define::{DefineError, DefineEvent};
use tdw_hooks::TransactionMode;

fn event_with_table(table: &str) -> DefineEvent {
    DefineEvent {
        event_name: "orders.created".to_string(),
        on_table: table.to_string(),
        hook_name: "notify.downstream".to_string(),
        transaction_mode: TransactionMode::PostCommit,
    }
}

#[test]
fn accepts_well_formed_schema_dot_table() {
    let event = event_with_table("public.orders");
    assert_eq!(event.validate(), Ok(()));
}

#[test]
fn accepts_underscores_and_digits_in_parts() {
    let event = event_with_table("schema_1.table_2");
    assert_eq!(event.validate(), Ok(()));
}

#[test]
fn rejects_table_without_separator() {
    // No '.' means `split_once` yields `None` -> invalid.
    let event = event_with_table("orders");
    assert_eq!(event.validate(), Err(DefineError::InvalidTableName));
}

#[test]
fn rejects_empty_table_name() {
    let event = event_with_table("");
    assert_eq!(event.validate(), Err(DefineError::InvalidTableName));
}

#[test]
fn rejects_more_than_two_parts() {
    // A second '.' must be rejected: `schema.table.extra` is not a 2-part name.
    let event = event_with_table("public.orders.extra");
    assert_eq!(event.validate(), Err(DefineError::InvalidTableName));
}

#[test]
fn rejects_empty_schema_part() {
    let event = event_with_table(".orders");
    assert_eq!(event.validate(), Err(DefineError::InvalidTableName));
}

#[test]
fn rejects_empty_table_part() {
    let event = event_with_table("public.");
    assert_eq!(event.validate(), Err(DefineError::InvalidTableName));
}

#[test]
fn rejects_illegal_characters_in_part() {
    // Hyphen is allowed in action names but not in table parts.
    let event = event_with_table("pub-lic.orders");
    assert_eq!(event.validate(), Err(DefineError::InvalidTableName));
}

#[test]
fn try_compile_hook_surfaces_table_name_error() {
    let event = event_with_table("no_separator");
    let err = event
        .try_compile_hook()
        .expect_err("invalid table name must fail compilation");
    assert_eq!(err, DefineError::InvalidTableName);
}

#[test]
fn try_compile_hook_succeeds_for_valid_table() {
    let event = event_with_table("analytics.events");
    let spec = event
        .try_compile_hook()
        .expect("valid event should compile");
    assert_eq!(spec.transaction_mode, TransactionMode::PostCommit);
}
