//! TCP transport listener (feature = "transport-tcp").
//!
//! Frame format: length-delimited JSON.
//! Each frame: `<u32 big-endian byte count><JSON bytes>`.
//! Client→server: `OpEnvelope`; server→client: `EventMsg`.

#![cfg(feature = "transport-tcp")]

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

/// Upper bound on concurrently-handled TCP connections. Each accepted
/// connection holds a permit for its lifetime; once this many are live, further
/// connections are rejected (closed immediately) instead of spawning unbounded
/// reader/writer tasks — a connection-exhaustion DoS guard (TT1).
const MAX_CONCURRENT_CONNECTIONS: usize = 256;

/// Spawn a TCP listener that forwards inbound `OpEnvelope` frames into `handle`
/// and streams `EventMsg` frames from `events` back to connected clients.
///
/// Each accepted connection gets:
/// - A reader task: reads length-delimited frames, deserialises each to
///   `OpEnvelope`, calls `handle.submit(env)`.
/// - A writer task: subscribes to a broadcast channel fed by `events` and
///   writes each serialised `EventMsg` back as a length-delimited frame.
///
/// Cancellation: returns when `cancel.cancelled()`.
///
/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub async fn serve_tcp(
    listener: TcpListener,
    handle: crate::SubmissionHandle,
    mut events: tokio::sync::mpsc::UnboundedReceiver<tdw_protocol::EventMsg>,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let (tx, _) = broadcast::channel::<String>(1024);

    // Pump EventMsgs into the broadcast channel so all connections receive them.
    let tx_for_pump = tx.clone();
    let cancel_for_pump = cancel.clone();
    let pump = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = cancel_for_pump.cancelled() => break,
                maybe = events.recv() => match maybe {
                    Some(event) => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            let _ = tx_for_pump.send(json);
                        }
                    }
                    None => break,
                }
            }
        }
    });

    let conn_limit = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _peer) = accept?;
                // Reject (close) the connection when at capacity rather than
                // spawning an unbounded task. The permit is held by the handler
                // for the connection's lifetime and released when it ends.
                let Ok(permit) = Arc::clone(&conn_limit).try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let conn_handle = handle.clone();
                let subscriber = tx.subscribe();
                let cancel_conn = cancel.clone();
                tokio::spawn(handle_tcp_conn(stream, conn_handle, subscriber, cancel_conn, permit));
            }
        }
    }

    pump.abort();
    Ok(())
}

async fn handle_tcp_conn(
    stream: TcpStream,
    handle: crate::SubmissionHandle,
    mut subscriber: broadcast::Receiver<String>,
    cancel: CancellationToken,
    // Held for the connection's lifetime; dropping it frees a slot in the
    // connection cap (TT1). Not otherwise used.
    _permit: OwnedSemaphorePermit,
) {
    let (mut read_half, mut write_half) = stream.into_split();

    let cancel_reader = cancel.clone();
    let reader = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = cancel_reader.cancelled() => return,
                res = read_frame(&mut read_half) => match res {
                    Ok(Some(bytes)) => {
                        if let Ok(env) = serde_json::from_slice::<tdw_protocol::OpEnvelope>(&bytes) {
                            let _ = handle.submit(env);
                        }
                    }
                    _ => return,
                }
            }
        }
    });

    let cancel_writer = cancel.clone();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = cancel_writer.cancelled() => return,
                res = subscriber.recv() => match res {
                    Ok(line) => {
                        // Frame length is a u32 prefix; a line larger than u32::MAX
                        // (4 GiB) cannot be framed, so close the writer rather than
                        // emit a truncated length. Broadcast lines are far smaller.
                        let Ok(len_u32) = u32::try_from(line.len()) else { return; };
                        let len = len_u32.to_be_bytes();
                        if write_half.write_all(&len).await.is_err() { return; }
                        if write_half.write_all(line.as_bytes()).await.is_err() { return; }
                        let _ = write_half.flush().await;
                    }
                    Err(_) => return,
                }
            }
        }
    });

    let _ = tokio::join!(reader, writer);
}

/// Read one length-delimited frame from `r`.
/// Returns `Ok(None)` on clean EOF; `Ok(Some(bytes))` on success; `Err` on IO error.
pub async fn read_frame<R: tokio::io::AsyncRead + Unpin>(
    r: &mut R,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > 16 * 1024 * 1024 {
        return Ok(None);
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}
