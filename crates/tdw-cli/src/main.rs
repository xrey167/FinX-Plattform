#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

mod ingest;
mod params;
mod reindex;
mod render;
mod routine;
mod status;
mod tree;

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

    // kg reindex: offline engine maintenance, no daemon connection
    // (knowledge-system B6).
    if args
        .windows(2)
        .any(|pair| pair[0] == "kg" && pair[1] == "reindex")
    {
        return reindex::run(&args).await;
    }

    // kg status: query daemon REST surface for KnowledgeRuntime observability
    // snapshot (knowledge-system K-E2).
    if args
        .windows(2)
        .any(|pair| pair[0] == "kg" && pair[1] == "status")
    {
        return status::run(&args).await;
    }

    // kg ingest: public ingestion surface through the KnowledgeIndexer seam
    // (knowledge-system K-E3). Reads JSONL from a file or stdin and writes
    // through the full B5 pipeline (idempotency, rules, lexical, graph).
    if args
        .windows(2)
        .any(|pair| pair[0] == "kg" && pair[1] == "ingest")
    {
        return ingest::run(&args).await;
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

    // No legacy subcommand and no args at all: keep the historical default —
    // connect and submit Op::Shutdown, then read until close or timeout. Without
    // a policy attached on the daemon side the response is EventMsg::Failed; this
    // is the unchanged pre-WS4 behaviour.
    if args.len() <= 1 {
        let events = connect_and_run(addr, Op::Shutdown).await?;
        for event in &events {
            println!(
                "{}",
                serde_json::to_string(event).map_err(|e| format!("serialize: {e}"))?
            );
        }
        return Ok(());
    }

    // WS4: catalog-driven command tree (`tdw <route...>`, `routes`, `routine`).
    dispatch_cli(&args).await
}

/// Parse `args` with the catalog-driven clap tree and run the matched command.
///
/// On a clap usage error this prints help/usage and exits via clap's own
/// non-zero exit, exactly like a derive-based CLI would.
async fn dispatch_cli(args: &[String]) -> Result<(), CliError> {
    let matches = tree::build_root().get_matches_from(args);
    match matches.subcommand() {
        Some(("routes", _)) => {
            print_routes();
            Ok(())
        }
        Some(("routine", sub)) => dispatch_routine(sub).await,
        Some((_, _)) => run_route(&matches).await,
        // `subcommand_required(true)` guarantees a subcommand; unreachable in
        // practice but handled without panicking.
        None => Err("no subcommand matched".into()),
    }
}

/// Print every catalog route with its kind and provider candidates.
fn print_routes() {
    let entries = tdw_endpoint_catalog::catalog();
    println!("{} catalog routes:", entries.len());
    for entry in &entries {
        let kind = match entry.kind {
            tdw_endpoint_catalog::EndpointKind::Fetch => "fetch",
            tdw_endpoint_catalog::EndpointKind::Compute => "compute",
        };
        let providers: Vec<&str> = entry.candidates.iter().map(|c| c.provider).collect();
        let providers = if providers.is_empty() {
            "(computed)".to_string()
        } else {
            providers.join(",")
        };
        println!("  {:<32} {:<8} {}", entry.route, kind, providers);
    }
}

/// Walk the matched subcommand chain to its leaf, returning the slash-joined
/// route, the leaf `ArgMatches`, and the catalog entry — or an error if the
/// resolved path is not a catalog route.
fn resolve_leaf(
    matches: &clap::ArgMatches,
) -> Result<
    (
        String,
        &clap::ArgMatches,
        tdw_endpoint_catalog::CatalogEntry,
    ),
    CliError,
> {
    let mut cursor = matches;
    let mut segments: Vec<String> = Vec::new();
    while let Some((name, sub)) = cursor.subcommand() {
        segments.push(name.to_string());
        cursor = sub;
    }
    let route = segments.join("/");
    let entry =
        tdw_endpoint_catalog::lookup(&route).ok_or_else(|| format!("unknown route: {route}"))?;
    Ok((route, cursor, entry))
}

/// Build and submit the `Op::FetchData` for a resolved route leaf, then render
/// the terminal event per the leaf's `--json` / `--export` flags.
async fn run_route(matches: &clap::ArgMatches) -> Result<(), CliError> {
    let (route, leaf, entry) = resolve_leaf(matches)?;
    let params_value = params::build_params(&entry, leaf);
    let addr = parse_addr(leaf)?;

    let op = Op::FetchData {
        route: route.clone(),
        params: params_value,
    };
    let events = connect_and_run(addr, op).await?;
    let terminal = terminal_result(&events);

    if leaf.get_flag("json") {
        let value = terminal.cloned().unwrap_or(serde_json::Value::Null);
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|e| format!("serialize: {e}"))?
        );
        return Ok(());
    }

    let records = terminal
        .map(render::records_from_result)
        .unwrap_or_default();

    if let Some(format) = leaf.get_one::<String>("export") {
        let path = export_records(&route, &records, leaf, format)?;
        println!("wrote {} rows to {path}", records.len());
        return Ok(());
    }

    println!("{}", render::table(&records));
    Ok(())
}

/// Resolve the `--addr` flag into a [`SocketAddr`].
fn parse_addr(matches: &clap::ArgMatches) -> Result<SocketAddr, CliError> {
    let raw = matches
        .get_one::<String>("addr")
        .map_or("127.0.0.1:7878", String::as_str);
    raw.parse()
        .map_err(|e| format!("invalid --addr {raw:?}: {e}").into())
}

/// Pull the `result` payload out of the terminal `EventMsg::Completed` event,
/// if the run completed; `None` for a failed / empty stream.
fn terminal_result(events: &[EventMsg]) -> Option<&serde_json::Value> {
    events.iter().rev().find_map(|event| match event {
        EventMsg::Completed { result, .. } => result.as_ref(),
        _ => None,
    })
}

/// Write `records` to a file in `format` (`csv` | `json`), returning the path.
fn export_records(
    route: &str,
    records: &[serde_json::Map<String, serde_json::Value>],
    matches: &clap::ArgMatches,
    format: &str,
) -> Result<String, CliError> {
    let default_name = format!("{}.{format}", route.replace('/', "_"));
    let path = matches
        .get_one::<String>("out")
        .cloned()
        .unwrap_or(default_name);
    let body = match format {
        "csv" => render::to_csv(records),
        "json" => render::to_json(records).map_err(|e| format!("json export: {e}"))?,
        other => return Err(format!("unsupported export format: {other}").into()),
    };
    std::fs::write(&path, body).map_err(|e| format!("write {path}: {e}"))?;
    Ok(path)
}

/// Dispatch the `routine record|run|list` subcommands.
async fn dispatch_routine(matches: &clap::ArgMatches) -> Result<(), CliError> {
    match matches.subcommand() {
        Some(("list", _)) => {
            for name in routine::list_routines()? {
                println!("{name}");
            }
            Ok(())
        }
        Some(("record", sub)) => routine_record(sub).await,
        Some(("run", sub)) => routine_run(sub).await,
        _ => Err("routine: expected record|run|list".into()),
    }
}

/// Run a wrapped route command and append its `OpEnvelope` to the routine file.
async fn routine_record(matches: &clap::ArgMatches) -> Result<(), CliError> {
    let name = matches
        .get_one::<String>("name")
        .ok_or("routine record: missing name")?;
    let command: Vec<String> = matches
        .get_many::<String>("command")
        .map(|values| values.cloned().collect())
        .unwrap_or_default();
    if command.is_empty() {
        return Err("routine record: provide a command after `--`".into());
    }

    // Re-parse the recorded command through the same tree to resolve its route.
    let mut argv = vec!["tdw".to_string()];
    argv.extend(command.iter().cloned());
    let sub_matches = tree::build_root()
        .try_get_matches_from(&argv)
        .map_err(|e| format!("routine record: {e}"))?;
    let (route, leaf, entry) = resolve_leaf(&sub_matches)?;
    let params_value = params::build_params(&entry, leaf);
    let addr = parse_addr(leaf)?;

    let op = Op::FetchData {
        route,
        params: params_value,
    };
    let envelope = build_envelope(op);
    let line = serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
    routine::append_envelope_line(name, &line)?;

    // Run the command now so `record` both stores and executes (parity with how
    // an OpenBB routine records a live invocation).
    let events = connect_and_run(addr, envelope.op).await?;
    let records = terminal_result(&events)
        .map(render::records_from_result)
        .unwrap_or_default();
    println!(
        "recorded {} ({} rows) -> {}",
        name,
        records.len(),
        routine::routine_path(name).display()
    );
    Ok(())
}

/// Replay every recorded envelope in a routine with fresh op/session ids and
/// optional `${k}` substitution.
async fn routine_run(matches: &clap::ArgMatches) -> Result<(), CliError> {
    let name = matches
        .get_one::<String>("name")
        .ok_or("routine run: missing name")?;
    let vars_raw: Vec<String> = matches
        .get_many::<String>("var")
        .map(|values| values.cloned().collect())
        .unwrap_or_default();
    let vars = routine::parse_vars(&vars_raw)?;
    let addr = parse_addr(matches)?;

    let envelopes = routine::read_envelopes(name)?;
    for (idx, stored) in envelopes.iter().enumerate() {
        let op = op_from_envelope_value(stored, &vars)?;
        let events = connect_and_run(addr, op).await?;
        let records = terminal_result(&events)
            .map(render::records_from_result)
            .unwrap_or_default();
        println!("step {} ({} rows)", idx + 1, records.len());
    }
    println!("ran {} step(s) from {name}", envelopes.len());
    Ok(())
}

/// Reconstruct an [`Op`] from a stored envelope JSON value, applying `${k}`
/// substitution to a `FetchData` op's params. The fresh op/session id is minted
/// by [`build_envelope`] at submit time, so only the op is rebuilt here.
fn op_from_envelope_value(
    stored: &serde_json::Value,
    vars: &std::collections::BTreeMap<String, String>,
) -> Result<Op, CliError> {
    let op_value = stored
        .get("op")
        .ok_or("routine envelope missing `op` field")?;
    let mut op: Op =
        serde_json::from_value(op_value.clone()).map_err(|e| format!("decode stored op: {e}"))?;
    if let Op::FetchData { params, .. } = &mut op {
        *params = routine::substitute(params, vars);
    }
    Ok(op)
}

/// Build a fresh `OpEnvelope` for `op` (new session + op id), matching the
/// envelope `connect_and_run` constructs.
fn build_envelope(op: Op) -> OpEnvelope {
    OpEnvelope::new(
        SessionId::generated(),
        1,
        ActorRef {
            actor_id: "user:tdw-cli".to_string(),
            kind: ActorKind::User,
            tenant_id: None,
        },
        op,
    )
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
        while let Ok(Some(bytes)) = read_frame(&mut stream).await {
            if let Ok(event) = serde_json::from_slice::<EventMsg>(&bytes) {
                events.push(event);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn connect_and_run_writes_shutdown_envelope_and_reads_event() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept client");
            let bytes = read_frame(&mut stream)
                .await
                .expect("read frame")
                .expect("client frame");
            let envelope: OpEnvelope =
                serde_json::from_slice(&bytes).expect("client envelope should decode");
            assert!(matches!(envelope.op, Op::Shutdown));

            let event = EventMsg::Completed {
                op_id: envelope.op_id,
                summary: Some("ok".to_string()),
                result: Some(serde_json::json!({ "ok": true })),
            };
            let json = serde_json::to_vec(&event).expect("serialize event");
            let len = u32::try_from(json.len()).expect("event len").to_be_bytes();
            stream.write_all(&len).await.expect("write len");
            stream.write_all(&json).await.expect("write body");
        });

        let events = connect_and_run(addr, Op::Shutdown)
            .await
            .expect("connect_and_run should succeed");
        server.await.expect("server task");

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], EventMsg::Completed { .. }));
    }
}
