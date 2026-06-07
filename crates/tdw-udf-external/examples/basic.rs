//! Offline `tdw-udf-external` example: validate an allowlisted external command
//! and show the path / shell-injection / timeout rejections.
//!
//! No process is spawned — this is the static validation gate only.
//!
//! ```text
//! cargo run --example tdw_udf_external_basic -p tdw-udf-external
//! ```

use tdw_udf_external::{
    ExternalUdfCommand, ExternalUdfError, MAX_TIMEOUT_MS, RUNTIME_NAME, validate_command,
};

fn main() {
    let command = ExternalUdfCommand {
        name: "tdw-udf-external".to_string(),
        command: "tdw-udf-runner".to_string(),
        args: vec!["--runtime".to_string(), "wasm".to_string()],
        timeout_ms: 5_000,
    };

    validate_command(&command).expect("allowlisted command should validate");
    println!("runtime: {RUNTIME_NAME}");
    println!("validated command: {} {:?}", command.command, command.args);

    // A path-bearing command is denied.
    let path = ExternalUdfCommand {
        command: "../runner".to_string(),
        ..command.clone()
    };
    assert_eq!(
        validate_command(&path),
        Err(ExternalUdfError::InvalidCommand)
    );

    // A shell-injecting argument is denied.
    let injection = ExternalUdfCommand {
        command: "tdw-runner".to_string(),
        args: vec!["symbol=AAPL;rm".to_string()],
        ..command.clone()
    };
    assert_eq!(
        validate_command(&injection),
        Err(ExternalUdfError::InvalidArgument)
    );

    // An over-cap timeout is denied.
    let slow = ExternalUdfCommand {
        command: "tdw-runner".to_string(),
        args: Vec::new(),
        timeout_ms: MAX_TIMEOUT_MS + 1,
        ..command
    };
    assert_eq!(
        validate_command(&slow),
        Err(ExternalUdfError::InvalidTimeout)
    );
    println!("path, shell-injection, and unbounded timeout are denied, as expected");
}
