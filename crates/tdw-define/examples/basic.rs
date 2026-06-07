//! Offline `tdw-define` example: declare a table-change event, compile it to a
//! hook spec, and show that an unsafe table name is rejected.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-define --example basic
//! ```

use tdw_define::{DefineError, DefineEvent};
use tdw_hooks::TransactionMode;

fn main() {
    // Declare: on a change to raw.market_data_bar, fire emit.market_data_changed
    // after the transaction commits.
    let define = DefineEvent {
        event_name: "market_data_changed".to_string(),
        on_table: "raw.market_data_bar".to_string(),
        hook_name: "emit.market_data_changed".to_string(),
        transaction_mode: TransactionMode::PostCommit,
    };

    // Meaningful operation: validate + compile into a runnable HookSpec.
    let hook = define
        .try_compile_hook()
        .expect("declaration should be valid");
    println!(
        "compiled hook: name={} mode={:?}",
        hook.name, hook.transaction_mode
    );
    println!("idempotency key: {}", define.idempotency_key());

    // An unsafe table name is rejected before any hook is produced.
    let unsafe_define = DefineEvent {
        on_table: "raw.market_data_bar;drop".to_string(),
        ..define
    };
    match unsafe_define.try_compile_hook() {
        Err(DefineError::InvalidTableName) => {
            println!("unsafe table name rejected: InvalidTableName");
        }
        other => println!("unexpected result: {other:?}"),
    }
}
