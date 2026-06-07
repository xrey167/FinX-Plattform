//! Offline, no-network example for `tdw-app-client`.
//!
//! Shows the two client surfaces without a live daemon:
//!  * `DaemonClientConfig` construction + validation, and a bounded, fail-fast
//!    `submit_and_wait` against a definitely-closed loopback port (so the result
//!    is a clean `Connect`/`TimedOut` error rather than a hang);
//!  * `AppClient` submitting in-process over a `tdw-app-server` `SubmissionHandle`.
//!
//! No async runtime is used — the client is blocking std I/O by design.
//!
//! Run with: `cargo run -p tdw-app-client --example tdw_app_client_basic`

use std::time::Duration;

use tdw_app_client::{
    AppClient, ClientInfo, DEFAULT_DAEMON_TCP_ADDR, DaemonClient, DaemonClientConfig,
    DaemonClientError,
};
use tdw_app_server::{DaemonEndpoint, DaemonTransport, channel};
use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};

fn shutdown_envelope() -> Result<OpEnvelope, Box<dyn std::error::Error>> {
    Ok(OpEnvelope::new(
        SessionId::new("session-example")?,
        1,
        ActorRef {
            actor_id: "user:example".to_string(),
            kind: ActorKind::User,
            tenant_id: None,
        },
        Op::Shutdown,
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Default config targets the loopback TCP daemon; validation passes.
    let config = DaemonClientConfig::default().with_timeout(Duration::from_millis(100));
    config.validate()?;
    println!(
        "default endpoint: {:?} {}",
        config.endpoint().transport,
        config.endpoint().address,
    );
    assert_eq!(config.endpoint().address, DEFAULT_DAEMON_TCP_ADDR);

    // 2. Submit to a closed port to show the bounded, fail-fast error path
    //    (no daemon is running, so this never hangs).
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    drop(listener); // the port is now closed
    let client = DaemonClient::new(
        DaemonClientConfig::tcp(addr.to_string()).with_timeout(Duration::from_millis(100)),
    );
    match client.submit_and_wait(&shutdown_envelope()?) {
        Err(DaemonClientError::Connect { address, .. }) => {
            println!("closed port -> Connect error for {address} (as expected)");
        }
        Err(DaemonClientError::TimedOut { action, .. }) => {
            println!("closed port -> TimedOut during {action} (as expected)");
        }
        other => println!("unexpected result: {other:?}"),
    }

    // 3. In-process client over a tdw-app-server submission handle.
    let (handle, _events, _agent_loop) = channel();
    let app_client = AppClient::try_new(
        ClientInfo {
            name: "tdw-example".to_string(),
            endpoint: DaemonEndpoint {
                transport: DaemonTransport::Tcp,
                address: DEFAULT_DAEMON_TCP_ADDR.to_string(),
            },
        },
        handle,
    )?;
    // `SubmissionError` is not `std::error::Error`, so expect rather than `?`.
    app_client
        .submit(shutdown_envelope()?)
        .expect("submission channel is open");
    println!(
        "in-process AppClient submitted as '{}'",
        app_client.info().name
    );

    Ok(())
}
