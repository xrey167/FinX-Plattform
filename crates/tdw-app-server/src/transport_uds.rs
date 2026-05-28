//! Unix Domain Socket transport listener (feature = "transport-uds", unix only).
//!
//! Mirrors `transport_tcp` but uses `tokio::net::UnixListener`.
//! Frame format is identical: `<u32 big-endian byte count><JSON bytes>`.

#![cfg(all(unix, feature = "transport-uds"))]

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// Spawn a UDS listener at `path` that forwards inbound `OpEnvelope` frames
/// into `handle` and streams `EventMsg` frames back to connected clients.
///
/// Removes an existing socket file at `path` before binding.
/// Cancellation: returns when `cancel.cancelled()`.
pub async fn serve_uds(
    path: std::path::PathBuf,
    handle: crate::SubmissionHandle,
    mut events: tokio::sync::mpsc::UnboundedReceiver<tdw_protocol::EventMsg>,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    // Remove stale socket file if present.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;

    let (tx, _) = broadcast::channel::<String>(1024);

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
                tokio::spawn(handle_uds_conn(stream, conn_handle, subscriber, cancel_conn));
            }
        }
    }

    pump.abort();
    // Best-effort cleanup.
    let _ = std::fs::remove_file(&path);
    Ok(())
}

async fn handle_uds_conn(
    stream: UnixStream,
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

async fn read_frame<R: tokio::io::AsyncRead + Unpin>(
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
