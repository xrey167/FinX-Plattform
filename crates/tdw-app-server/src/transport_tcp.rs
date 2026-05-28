//! TCP transport listener (feature = "transport-tcp").
//!
//! Frame format: length-delimited JSON.
//! Each frame: `<u32 big-endian byte count><JSON bytes>`.
//! Client→server: `OpEnvelope`; server→client: `EventMsg`.

#![cfg(feature = "transport-tcp")]

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

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
                _ = cancel_for_pump.cancelled() => break,
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

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _peer) = accept?;
                let conn_handle = handle.clone();
                let subscriber = tx.subscribe();
                let cancel_conn = cancel.clone();
                tokio::spawn(handle_tcp_conn(stream, conn_handle, subscriber, cancel_conn));
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
) {
    let (mut read_half, mut write_half) = stream.into_split();

    let cancel_reader = cancel.clone();
    let reader = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = cancel_reader.cancelled() => return,
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
                _ = cancel_writer.cancelled() => return,
                res = subscriber.recv() => match res {
                    Ok(line) => {
                        let len = (line.len() as u32).to_be_bytes();
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
pub(crate) async fn read_frame<R: tokio::io::AsyncRead + Unpin>(
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
