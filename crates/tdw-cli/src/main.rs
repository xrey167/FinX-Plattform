#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::time::Duration;

use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};
use tdw_test_utils::smoke::{allocate_storage_root, run_end_to_end_smoke};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub type CliError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), CliError> {
    let args: Vec<String> = std::env::args().collect();

    // --smoke: run G009 offline smoke and print report.
    if args.iter().any(|a| a == "--smoke") {
        let symbol = args
            .iter()
            .position(|a| a == "--smoke")
            .and_then(|i| args.get(i + 1))
            .map_or("AAPL", |s| s.as_str());
        let root = allocate_storage_root("tdw-cli-smoke");
        let report = run_end_to_end_smoke(symbol, root.clone())
            .await
            .map_err(|e| format!("smoke error: {e}"))?;
        println!(
            "tdw-cli provider={} endpoint={} symbol={} rows={} blob={} bytes={} roundtrip={}",
            report.provider,
            report.endpoint,
            report.query_symbol,
            report.rows_fetched,
            report.blob_key,
            report.blob_bytes_written,
            report.roundtrip_ok,
        );
        let _ = std::fs::remove_dir_all(&root);
        return Ok(());
    }

    // Determine daemon address (default TCP loopback matching tdw-service default).
    let addr: SocketAddr = "127.0.0.1:7878".parse().expect("static addr");

    // Sub-command dispatch.
    if let Some(pos) = args.iter().position(|a| a == "run-query") {
        let sql = args
            .get(pos + 1)
            .cloned()
            .unwrap_or_else(|| "select 1".to_string());
        let op = Op::RunQuery {
            sql,
            plan_id: None,
            cost_hint: None,
        };
        let events = connect_and_run(addr, op).await?;
        for event in &events {
            println!(
                "{}",
                serde_json::to_string(event).map_err(|e| format!("serialize: {e}"))?
            );
        }
        return Ok(());
    }

    // Default mode: connect and submit Op::Shutdown then read until close or timeout.
    // Note: without a policy attached on the daemon side, the response will be
    // EventMsg::Failed — this is expected behaviour until P7 attaches a policy.
    let events = connect_and_run(addr, Op::Shutdown).await?;
    for event in &events {
        println!(
            "{}",
            serde_json::to_string(event).map_err(|e| format!("serialize: {e}"))?
        );
    }

    Ok(())
}

/// Connect to `addr`, submit `op` as a length-delimited JSON frame, then read
/// `EventMsg` frames until the connection closes or a 5-second timeout elapses.
///
/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub async fn connect_and_run(addr: SocketAddr, op: Op) -> Result<Vec<EventMsg>, CliError> {
    let mut stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
        .await
        .map_err(|_| "connect timeout")?
        .map_err(|e| format!("connect failed ({addr}): {e}"))?;

    // Build envelope.
    let session_id = SessionId::generated();
    let envelope = OpEnvelope::new(
        session_id,
        1,
        ActorRef {
            actor_id: "user:tdw-cli".to_string(),
            kind: ActorKind::User,
            tenant_id: None,
        },
        op,
    );

    // Write length-delimited frame.
    write_frame(&mut stream, &envelope).await?;

    // Read EventMsg frames until EOF or timeout.
    let mut events = Vec::new();
    let deadline = Duration::from_secs(5);
    let result = tokio::time::timeout(deadline, async {
        loop {
            match read_frame(&mut stream).await {
                Ok(Some(bytes)) => {
                    if let Ok(event) = serde_json::from_slice::<EventMsg>(&bytes) {
                        events.push(event);
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    })
    .await;

    // Timeout is acceptable — we may have received all relevant events.
    let _ = result;

    Ok(events)
}

async fn write_frame(stream: &mut TcpStream, envelope: &OpEnvelope) -> Result<(), CliError> {
    let json = serde_json::to_vec(envelope).map_err(|e| format!("serialize envelope: {e}"))?;
    let len = u32::try_from(json.len())
        .map_err(|_| "envelope length exceeds u32 frame".to_string())?
        .to_be_bytes();
    stream
        .write_all(&len)
        .await
        .map_err(|e| format!("write len: {e}"))?;
    stream
        .write_all(&json)
        .await
        .map_err(|e| format!("write body: {e}"))?;
    stream.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

async fn read_frame(stream: &mut TcpStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > 16 * 1024 * 1024 {
        return Ok(None);
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(Some(buf))
}
