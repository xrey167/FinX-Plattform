use std::env;
use std::fs;
use std::path::{Path, PathBuf};

mod improve_scan;
mod openapi;

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".to_string());
    let result = match command.as_str() {
        "bench" => bench(),
        "bench-compare" => bench_compare(args.next()),
        "quality-gate" => quality_gate(args.next().as_deref()),
        "ddl-export" => ddl_export(args.next().as_deref()),
        "migrate" => match args.next().as_deref() {
            Some("up") => migrate_up(),
            Some("down") => migrate_down(),
            Some("status") => migrate_status(),
            _ => help(),
        },
        "schema-sync" => schema_sync(),
        "events" => match args.next().as_deref() {
            Some("schema-check") => events_schema_check(),
            _ => help(),
        },
        "protocol" => match args.next().as_deref() {
            Some("schema-check") => protocol_schema_check(),
            _ => help(),
        },
        "config" => match args.next().as_deref() {
            Some("schema-check") => config_schema_check(),
            _ => help(),
        },
        "mutation" => match args.next().as_deref() {
            Some("changed") => mutation_changed(args.next().as_deref()),
            Some("report") => mutation_report(args.next()),
            _ => help(),
        },
        "catalog-check" => catalog_check(),
        "openapi-sync" => openapi::openapi_sync(),
        "openapi-check" => openapi::openapi_check(),
        "clean-room-audit" => clean_room_audit(),
        "crate-readiness-check" => crate_readiness_check(),
        "prerelease-check" => prerelease_check(),
        "improve-scan" => improve_scan::improve_scan(),
        _ => help(),
    };

    if let Err(error) = result {
        eprintln!("xtask error: {error}");
        std::process::exit(1);
    }
}

#[allow(clippy::unnecessary_wraps)] // sibling arm of the unified `match` in main(); must share Result<(), String> with arms (quality_gate/ddl_export/schema_sync) that genuinely return Err
fn help() -> Result<(), String> {
    println!(
        "xtask commands: bench | bench-compare <baseline> | quality-gate <write|check> | ddl-export <postgres|clickhouse> | migrate <up|down|status> | schema-sync | events schema-check | protocol schema-check | config schema-check | mutation <changed [--run]|report [out-dir]> | catalog-check | openapi-sync | openapi-check | clean-room-audit | crate-readiness-check | prerelease-check | improve-scan"
    );
    Ok(())
}

/// Path the benchmark run writes and the ratchet reads as the current sample.
const PERF_HISTORY_PATH: &str = "docs/perf-history.json";

/// Default baseline the ratchet compares against when none is supplied.
const DEFAULT_PERF_BASELINE_PATH: &str = "docs/perf-baseline.json";

/// Regression tolerance: a workload fails enforcement once the current
/// `ns_per_iter` exceeds the baseline by more than this factor. Deliberately
/// generous (25%) so ordinary CI scheduling noise does not flap the gate.
const REGRESSION_TOLERANCE: f64 = 1.25;

/// A single measured workload result, written to `docs/perf-history.json`.
struct WorkloadResult {
    name: &'static str,
    ns_per_iter: f64,
    iters: u64,
}

/// Run the deterministic, pure-CPU benchmark suite and write the results to
/// `docs/perf-history.json`.
///
/// Every workload exercises only public `tdw-proto` types via prost
/// `Message::{encode, decode}` over fixed inputs, so the numbers depend only on
/// the host CPU — never on wall-clock, randomness, I/O, or allocation patterns
/// that differ run to run. Each workload warms up, then auto-sizes a per-batch
/// iteration count so a batch runs ~20-50ms, takes five measured batches, and
/// reports the median `ns_per_iter` across those batches to damp scheduler
/// noise.
fn bench() -> Result<(), String> {
    let results = [
        bench_proto_encode_ohlcv(),
        bench_proto_decode_ohlcv(),
        bench_proto_roundtrip_envelope(),
    ];

    println!("{:<28} {:>14} {:>12}", "workload", "ns_per_iter", "iters");
    for result in &results {
        println!(
            "{:<28} {:>14.1} {:>12}",
            result.name, result.ns_per_iter, result.iters
        );
    }

    let workloads: Vec<_> = results
        .iter()
        .map(|result| {
            serde_json::json!({
                "name": result.name,
                "ns_per_iter": result.ns_per_iter,
                "iters": result.iters,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "schema": 1,
        "workloads": workloads,
    });
    let content = serde_json::to_string_pretty(&payload)
        .map(|content| content + "\n")
        .map_err(|error| error.to_string())?;

    fs::create_dir_all("docs").map_err(|error| error.to_string())?;
    fs::write(PERF_HISTORY_PATH, content).map_err(|error| error.to_string())?;
    println!("bench wrote {PERF_HISTORY_PATH}");
    Ok(())
}

/// A fixed `OhlcvBar` used as the deterministic input across encode/decode/
/// envelope workloads.
fn fixture_bar() -> tdw_proto::OhlcvBar {
    tdw_proto::OhlcvBar {
        symbol: "AAPL".to_string(),
        provider: "polygon".to_string(),
        timeframe: "1m".to_string(),
        ts_ns: 1_700_000_000_000_000_000_i64,
        open: 150.25,
        high: 151.50,
        low: 149.80,
        close: 151.00,
        volume: 1_234_567.0,
        vwap: 150.75,
        trade_count: 42_000,
    }
}

/// `proto_encode_ohlcv`: encode a fixed `OhlcvBar` into a reused buffer per iter.
fn bench_proto_encode_ohlcv() -> WorkloadResult {
    use prost::Message;
    let bar = fixture_bar();
    let mut buf = Vec::with_capacity(64);
    let ns_per_iter = measure_workload(|iters| {
        for _ in 0..iters {
            buf.clear();
            std::hint::black_box(&bar)
                .encode(&mut buf)
                .expect("encode must not fail");
            std::hint::black_box(&buf);
        }
    });
    WorkloadResult {
        name: "proto_encode_ohlcv",
        ns_per_iter: ns_per_iter.0,
        iters: ns_per_iter.1,
    }
}

/// `proto_decode_ohlcv`: decode a pre-encoded `OhlcvBar` per iter.
fn bench_proto_decode_ohlcv() -> WorkloadResult {
    use prost::Message;
    let bar = fixture_bar();
    let mut bytes = Vec::with_capacity(64);
    bar.encode(&mut bytes).expect("encode must not fail");
    let ns_per_iter = measure_workload(|iters| {
        for _ in 0..iters {
            let decoded = tdw_proto::OhlcvBar::decode(std::hint::black_box(bytes.as_slice()))
                .expect("decode must not fail");
            std::hint::black_box(decoded);
        }
    });
    WorkloadResult {
        name: "proto_decode_ohlcv",
        ns_per_iter: ns_per_iter.0,
        iters: ns_per_iter.1,
    }
}

/// `proto_roundtrip_envelope`: encode+decode a fixed `MarketDataEnvelope`
/// (carrying a bar payload) per iter.
fn bench_proto_roundtrip_envelope() -> WorkloadResult {
    use prost::Message;
    let envelope = tdw_proto::MarketDataEnvelope {
        provider: "polygon".to_string(),
        symbol: "AAPL".to_string(),
        ingestion_id: "01HZ000000000000000000001".to_string(),
        received_at_ns: 1_700_000_003_000_100_000_i64,
        payload: Some(tdw_proto::Payload::Bar(fixture_bar())),
    };
    let mut buf = Vec::with_capacity(128);
    let ns_per_iter = measure_workload(|iters| {
        for _ in 0..iters {
            buf.clear();
            std::hint::black_box(&envelope)
                .encode(&mut buf)
                .expect("encode must not fail");
            let decoded =
                tdw_proto::MarketDataEnvelope::decode(std::hint::black_box(buf.as_slice()))
                    .expect("decode must not fail");
            std::hint::black_box(decoded);
        }
    });
    WorkloadResult {
        name: "proto_roundtrip_envelope",
        ns_per_iter: ns_per_iter.0,
        iters: ns_per_iter.1,
    }
}

/// Warm up, auto-size the per-batch iteration count to ~20-50ms, run five
/// measured batches, and return `(median ns_per_iter, per_batch_iters)`.
///
/// `run` executes the workload `iters` times; it must `black_box` its inputs
/// and outputs so the optimizer cannot hoist the work out of the loop.
fn measure_workload<F: FnMut(u64)>(mut run: F) -> (f64, u64) {
    use std::time::Instant;

    // Warm up caches / branch predictors and force first-touch allocation.
    run(1_000);

    // Auto-size: grow the batch until one batch takes at least 20ms, capping
    // the target near the middle of the 20-50ms window.
    let target = std::time::Duration::from_millis(30);
    let mut iters: u64 = 1_000;
    loop {
        let start = Instant::now();
        run(iters);
        let elapsed = start.elapsed();
        if elapsed >= std::time::Duration::from_millis(20) || iters >= 1u64 << 32 {
            break;
        }
        // Scale toward the target, at least doubling, to converge quickly.
        let scale = if elapsed.as_nanos() == 0 {
            8
        } else {
            ((target.as_nanos() / elapsed.as_nanos()) as u64).max(2)
        };
        iters = iters.saturating_mul(scale);
    }

    let mut samples = Vec::with_capacity(5);
    for _ in 0..5 {
        let start = Instant::now();
        run(iters);
        let elapsed = start.elapsed();
        #[allow(clippy::cast_precision_loss)]
        let ns_per_iter = elapsed.as_nanos() as f64 / iters as f64;
        samples.push(ns_per_iter);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("benchmark timings are finite"));
    (samples[samples.len() / 2], iters)
}

/// Compare the current `docs/perf-history.json` against a baseline and enforce
/// the regression ratchet.
///
/// Bootstrap mode (baseline marked `"bootstrap": true` or with no workloads)
/// just prints the current numbers and returns OK so the gate never fails
/// before a real, CI-measured baseline has been committed. Enforcing mode fails
/// if any baseline workload regressed beyond [`REGRESSION_TOLERANCE`]; a missing
/// current workload warns rather than fails.
fn bench_compare(baseline: Option<String>) -> Result<(), String> {
    let baseline_path = baseline.unwrap_or_else(|| DEFAULT_PERF_BASELINE_PATH.to_string());

    let current = read_perf_json(PERF_HISTORY_PATH)?;
    let baseline = read_perf_json(&baseline_path)?;

    let bootstrap = baseline
        .get("bootstrap")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let baseline_workloads = baseline
        .get("workloads")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    if bootstrap || baseline_workloads.is_empty() {
        println!("bench-compare: bootstrap baseline at {baseline_path}; current run:");
        print_perf_table(&current);
        println!(
            "bench-compare: commit {PERF_HISTORY_PATH} as {baseline_path} to enable enforcement"
        );
        return Ok(());
    }

    let current_workloads = current
        .get("workloads")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    println!(
        "{:<28} {:>14} {:>14} {:>9} status",
        "workload", "baseline", "current", "delta%"
    );

    let mut regressions = Vec::new();
    for baseline_workload in &baseline_workloads {
        let Some(name) = baseline_workload
            .get("name")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let baseline_ns = workload_ns_per_iter(baseline_workload);
        let current_workload = current_workloads.iter().find(|workload| {
            workload.get("name").and_then(serde_json::Value::as_str) == Some(name)
        });
        let Some(current_workload) = current_workload else {
            let dash = "-";
            println!("{name:<28} {baseline_ns:>14.1} {dash:>14} {dash:>9} MISSING");
            continue;
        };
        let current_ns = workload_ns_per_iter(current_workload);
        let delta_pct = if baseline_ns == 0.0 {
            0.0
        } else {
            (current_ns - baseline_ns) / baseline_ns * 100.0
        };
        let regressed = current_ns > baseline_ns * REGRESSION_TOLERANCE;
        let status = if regressed { "REGRESSED" } else { "OK" };
        println!("{name:<28} {baseline_ns:>14.1} {current_ns:>14.1} {delta_pct:>+9.1} {status}");
        if regressed {
            regressions.push(name.to_string());
        }
    }

    if regressions.is_empty() {
        println!("bench-compare: PASS (no workload regressed beyond {REGRESSION_TOLERANCE:.2}x)");
        Ok(())
    } else {
        Err(format!(
            "bench-compare: FAIL; regressed beyond {:.0}% tolerance: {}",
            (REGRESSION_TOLERANCE - 1.0) * 100.0,
            regressions.join(", ")
        ))
    }
}

/// Read and parse a perf JSON document (`perf-history.json` / baseline).
fn read_perf_json(path: &str) -> Result<serde_json::Value, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("perf file missing or unreadable at {path}: {error}"))?;
    serde_json::from_str(&content).map_err(|error| format!("invalid perf JSON at {path}: {error}"))
}

/// Extract `ns_per_iter` from a workload object, defaulting to 0 when absent.
fn workload_ns_per_iter(workload: &serde_json::Value) -> f64 {
    workload
        .get("ns_per_iter")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
}

/// Print the workloads in a perf document as a human-readable table.
fn print_perf_table(perf: &serde_json::Value) {
    println!("{:<28} {:>14} {:>12}", "workload", "ns_per_iter", "iters");
    if let Some(workloads) = perf.get("workloads").and_then(serde_json::Value::as_array) {
        for workload in workloads {
            let name = workload
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            let ns = workload_ns_per_iter(workload);
            let iters = workload
                .get("iters")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            println!("{name:<28} {ns:>14.1} {iters:>12}");
        }
    }
}

fn quality_gate(mode: Option<&str>) -> Result<(), String> {
    match mode.unwrap_or("write") {
        "write" => write_quality_gate(),
        "check" => check_quality_gate(),
        other => Err(format!("unknown quality-gate mode: {other}")),
    }
}

fn write_quality_gate() -> Result<(), String> {
    let path = quality_gate_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&path, quality_gate_json()?).map_err(|error| error.to_string())?;
    println!("quality-gate wrote {}", path.display());
    Ok(())
}

fn check_quality_gate() -> Result<(), String> {
    let path = quality_gate_path();
    let expected = quality_gate_json()?;
    let current = fs::read_to_string(&path).map_err(|error| {
        format!(
            "quality-gate artifact missing or unreadable at {}: {error}",
            path.display()
        )
    })?;
    if current != expected {
        return Err(format!(
            "quality-gate artifact is stale; run `cargo run -p xtask -- quality-gate write` and commit {}",
            path.display()
        ));
    }
    println!("quality-gate checked {}", path.display());
    Ok(())
}

fn quality_gate_path() -> PathBuf {
    PathBuf::from("docs/quality/phase-exit-gates.json")
}

fn quality_gate_json() -> Result<String, String> {
    let gates: Vec<_> = quality_gates()
        .iter()
        .map(|gate| {
            serde_json::json!({
                "id": gate.id,
                "tier": gate.tier,
                "command": gate.command,
                "artifact": gate.artifact,
                "cadence": gate.cadence,
                "requiredForPhaseExit": gate.required_for_phase_exit,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "version": 1,
        "generatedBy": "cargo run -p xtask -- quality-gate write",
        "policy": {
            "narrowBeforeBroad": true,
            "checkpointRequiresCommandsAndArtifacts": true,
            "failedGateRequiresBlockerEvidence": true,
            "testQuarantineAllowedAtRelease": false
        },
        "gates": gates
    });
    serde_json::to_string_pretty(&payload)
        .map(|content| content + "\n")
        .map_err(|error| error.to_string())
}

#[derive(Clone, Copy, Debug)]
struct QualityGate {
    id: &'static str,
    tier: &'static str,
    command: &'static str,
    artifact: &'static str,
    cadence: &'static str,
    required_for_phase_exit: bool,
}

const fn quality_gates() -> &'static [QualityGate] {
    &[
        QualityGate {
            id: "fmt",
            tier: "lint",
            command: "just fmt-check",
            artifact: "rustfmt stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "lint",
            tier: "lint",
            command: "just lint",
            artifact: "clippy stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "unit",
            tier: "test",
            command: "just test-unit",
            artifact: "cargo test stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "integration",
            tier: "test",
            command: "just test-integration",
            artifact: "cargo test stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "property",
            tier: "test",
            command: "just test-property",
            artifact: "cargo test stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "e2e-subset",
            tier: "test",
            command: "just test-e2e",
            artifact: "cargo test stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "adversarial",
            tier: "security",
            command: "just test-adversarial",
            artifact: "cargo test stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "coverage",
            tier: "coverage",
            command: "just coverage",
            artifact: "lcov.info",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "schema-sync",
            tier: "schema",
            command: "just schema-sync",
            artifact: "docs/schemas/agent/*.schema.json",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "event-schema-check",
            tier: "schema",
            command: "just event-schema-check",
            artifact: "docs/schemas/event/*.schema.json",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "perf",
            tier: "performance",
            command: "just bench",
            artifact: "docs/perf-history.json",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "dependency-audit",
            tier: "security",
            command: "just deny",
            artifact: "cargo-deny stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "clean-room-audit",
            tier: "governance",
            command: "just audit",
            artifact: "xtask clean-room-audit stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "windows-release",
            tier: "release",
            command: "just windows-release",
            artifact: "MSVC release build stdout",
            cadence: "phase-exit",
            required_for_phase_exit: true,
        },
        QualityGate {
            id: "mutation-smoke",
            tier: "mutation",
            command: "just mutation-core",
            artifact: "cargo-mutants stdout",
            cadence: "nightly",
            required_for_phase_exit: false,
        },
        QualityGate {
            id: "mutation-summary",
            tier: "mutation",
            command: "cargo run -p xtask -- mutation report",
            artifact: "mutation-summary.json (CI artifact)",
            cadence: "nightly",
            required_for_phase_exit: false,
        },
        QualityGate {
            id: "e2e-full",
            tier: "test",
            command: "cargo test --workspace --features e2e",
            artifact: "cargo test stdout",
            cadence: "nightly",
            required_for_phase_exit: false,
        },
        QualityGate {
            id: "flaky-detect",
            tier: "stability",
            command: "nightly integration/e2e x10",
            artifact: "nightly workflow log",
            cadence: "nightly",
            required_for_phase_exit: false,
        },
    ]
}

fn ddl_export(target: Option<&str>) -> Result<(), String> {
    let target = match target {
        Some("postgres") | None => tdw_sql_codegen::SqlTarget::Postgres,
        Some("clickhouse") => tdw_sql_codegen::SqlTarget::ClickHouse,
        Some(other) => return Err(format!("unknown ddl target: {other}")),
    };
    print!("{}", tdw_sql_codegen::export_domain_ddl(target));
    Ok(())
}

#[allow(clippy::unnecessary_wraps)] // sibling of migrate_* match arms (and the top-level dispatch) that must share Result<(), String>; uniform dispatch requires the wrapper
fn migrate_up() -> Result<(), String> {
    println!(
        "offline migrate up plan: {}",
        tdw_migration::migration_status()
    );
    for migration in tdw_migration::postgres_migrations()
        .into_iter()
        .chain(tdw_migration::clickhouse_migrations())
    {
        println!("apply {} {}", migration.version, migration.name);
    }
    Ok(())
}

#[allow(clippy::unnecessary_wraps)] // sibling of migrate_* match arms (and the top-level dispatch) that must share Result<(), String>; uniform dispatch requires the wrapper
fn migrate_down() -> Result<(), String> {
    println!("offline migrate down plan: no destructive migration is run by xtask scaffold");
    Ok(())
}

#[allow(clippy::unnecessary_wraps)] // sibling of migrate_* match arms (and the top-level dispatch) that must share Result<(), String>; uniform dispatch requires the wrapper
fn migrate_status() -> Result<(), String> {
    println!("{}", tdw_migration::migration_status());
    Ok(())
}

fn schema_sync() -> Result<(), String> {
    let schema_dir = Path::new("docs/schemas/agent");
    fs::create_dir_all(schema_dir).map_err(|error| error.to_string())?;
    let bundle = tdw_agent::schema_bundle();
    for (name, schema) in &bundle {
        let content =
            serde_json::to_string_pretty(schema).map_err(|error| error.to_string())? + "\n";
        fs::write(schema_dir.join(format!("{name}.schema.json")), content)
            .map_err(|error| error.to_string())?;
    }
    println!(
        "schema-sync wrote {} agent schemas to {}",
        bundle.len(),
        schema_dir.display()
    );
    Ok(())
}

fn events_schema_check() -> Result<(), String> {
    let count = write_schema_bundle(
        Path::new("docs/schemas/event"),
        &tdw_event::event_schema_bundle(),
    )?;
    println!("events schema-check wrote {count} event schemas to docs/schemas/event");
    Ok(())
}

fn protocol_schema_check() -> Result<(), String> {
    let count = write_schema_bundle(
        Path::new("docs/schemas/protocol"),
        &tdw_protocol::schema_bundle(),
    )?;
    println!("protocol schema-check wrote {count} protocol schemas to docs/schemas/protocol");
    Ok(())
}

fn config_schema_check() -> Result<(), String> {
    let count = write_schema_bundle(
        Path::new("docs/schemas/config"),
        &tdw_config::schema_bundle(),
    )?;
    println!("config schema-check wrote {count} config schemas to docs/schemas/config");
    Ok(())
}

fn write_schema_bundle(
    schema_dir: &Path,
    bundle: &std::collections::BTreeMap<&'static str, serde_json::Value>,
) -> Result<usize, String> {
    fs::create_dir_all(schema_dir).map_err(|error| error.to_string())?;
    for (name, schema) in bundle {
        let content =
            serde_json::to_string_pretty(schema).map_err(|error| error.to_string())? + "\n";
        fs::write(schema_dir.join(format!("{name}.schema.json")), content)
            .map_err(|error| error.to_string())?;
    }
    Ok(bundle.len())
}

/// Crates that always receive a scoped mutation run, regardless of the diff.
///
/// This is the foundational/protocol/daemon/storage/worker union called out in
/// ADR 0014 and `TEST-POLICY-001`/`002`: protocol and daemon transport
/// boundaries, the core domain crate, the storage router, and the worker.
const MUTATION_BASELINE_CRATES: &[&str] = &[
    "tdw-core",
    "tdw-protocol",
    "tdw-app-client",
    "tdw-mcp",
    "tdw-worker",
    "tdw-storage-router",
];

/// Resolve the workspace member package name for a changed path, if any.
///
/// Paths under `crates/<name>/` map to package `<name>`; the `xtask/` crate is
/// intentionally skipped because it is not part of the mutation scope.
fn crate_for_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let rest = normalized.strip_prefix("crates/")?;
    let name = rest.split('/').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Crates changed versus `origin/main` according to `git diff`.
///
/// Returns `Ok(None)` when `git` is unavailable or the diff cannot be computed,
/// so callers can degrade to a baseline-only plan instead of failing.
fn changed_crates_vs_main() -> Option<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", "origin/main...HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut crates: Vec<String> = stdout
        .lines()
        .filter_map(crate_for_path)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    crates.sort();
    Some(crates)
}

/// Whether the `cargo-mutants` binary is discoverable on PATH.
fn cargo_mutants_available() -> bool {
    std::process::Command::new("cargo")
        .args(["mutants", "--version"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// TEST-POLICY-002: print (and optionally run) a scoped `cargo mutants` plan for
/// crates changed versus `origin/main`, unioned with the baseline crate set.
///
/// Plan-only by default and offline-friendly: it always prints the scoped
/// invocation plan and exits `Ok`, so it never depends on `git` or
/// `cargo-mutants` being available. Pass `--run` (`mutation changed --run`) to
/// actually execute the sweep; that path requires `cargo-mutants` and fails on
/// unclassified survivors. Skip / `MUTANT-EQUIV` annotations are honored by
/// `cargo-mutants` itself via in-source `// cargo-mutants: skip` /
/// `mutants::skip` markers, so no extra flags are needed here. This stays
/// outside phase-exit gates.
fn mutation_changed(flag: Option<&str>) -> Result<(), String> {
    let run = matches!(flag, Some("--run" | "run"));
    let changed = changed_crates_vs_main();
    match &changed {
        Some(list) if list.is_empty() => {
            println!("mutation changed: no crate-scoped changes detected vs origin/main");
        }
        Some(list) => {
            println!(
                "mutation changed: changed crates vs origin/main: {}",
                list.join(", ")
            );
        }
        None => {
            println!(
                "mutation changed: git diff unavailable (offline or detached); using baseline crates only"
            );
        }
    }

    let mut scope: std::collections::BTreeSet<String> = MUTATION_BASELINE_CRATES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    if let Some(list) = changed {
        scope.extend(list);
    }
    let scope: Vec<String> = scope.into_iter().collect();

    println!("mutation changed: scoped plan ({} crates):", scope.len());
    for name in &scope {
        let features = if name == "tdw-core" {
            " --features inventory-registration"
        } else {
            ""
        };
        println!("  cargo mutants -p {name}{features} --timeout 120");
    }

    if !run {
        println!(
            "mutation changed: plan-only (pass `--run` to execute the scoped sweep with cargo-mutants)"
        );
        return Ok(());
    }

    if !cargo_mutants_available() {
        return Err(
            "mutation changed --run requires cargo-mutants (install via `cargo install cargo-mutants` or taiki-e/install-action)"
                .to_string(),
        );
    }

    println!("mutation changed: cargo-mutants detected; running scoped sweep");
    let mut survivors = Vec::new();
    for name in &scope {
        let mut command = std::process::Command::new("cargo");
        command.arg("mutants").arg("-p").arg(name);
        if name == "tdw-core" {
            command.args(["--features", "inventory-registration"]);
        }
        command.args(["--timeout", "120"]);
        let status = command.status().map_err(|error| error.to_string())?;
        if !status.success() {
            survivors.push(name.clone());
        }
    }

    if survivors.is_empty() {
        println!("mutation changed: no unclassified survivors");
        Ok(())
    } else {
        Err(format!(
            "mutation changed: unclassified survivors in: {}",
            survivors.join(", ")
        ))
    }
}

/// TEST-POLICY-001: aggregate per-crate `cargo-mutants` `outcomes.json` files
/// into a single machine-readable `mutation-summary.json` for CI upload.
///
/// `out_dir` defaults to `mutants.out` (the cargo-mutants default). The function
/// walks the tree for any `outcomes.json` (supporting both `<crate>/outcomes.json`
/// and the `<crate>/mutants.out/outcomes.json` layout produced by
/// `cargo mutants --output <crate>`), extracts per-crate runtime, killed,
/// survivors (missed/caught classification), and timeout counts, and writes the
/// summary next to the inputs. It is report-only: missing inputs degrade to an
/// empty-but-valid summary rather than failing.
fn mutation_report(out_dir: Option<String>) -> Result<(), String> {
    let root = PathBuf::from(out_dir.unwrap_or_else(|| "mutants.out".to_string()));
    let mut crates = Vec::new();

    let mut outcome_files: Vec<PathBuf> = Vec::new();
    find_outcomes_files(&root, &mut outcome_files)?;
    let mut outcome_files: Vec<(String, PathBuf)> = outcome_files
        .into_iter()
        .map(|path| (outcomes_crate_label(&root, &path), path))
        .collect();
    outcome_files.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, path) in &outcome_files {
        let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
        let parsed: serde_json::Value =
            serde_json::from_str(&content).map_err(|error| error.to_string())?;
        crates.push(summarize_outcomes(name, &parsed));
    }

    let summary = serde_json::json!({
        "version": 1,
        "generatedBy": "cargo run -p xtask -- mutation report",
        "scoredFloorEnforced": false,
        "source": root.display().to_string(),
        "crates": crates,
    });
    let payload = serde_json::to_string_pretty(&summary)
        .map(|content| content + "\n")
        .map_err(|error| error.to_string())?;

    let summary_path = root.join("mutation-summary.json");
    if let Some(parent) = summary_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&summary_path, &payload).map_err(|error| error.to_string())?;
    print!("{payload}");
    println!("mutation report wrote {}", summary_path.display());
    Ok(())
}

/// Recursively collect every `outcomes.json` under `dir`.
fn find_outcomes_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            find_outcomes_files(&path, files)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("outcomes.json") {
            files.push(path);
        }
    }
    Ok(())
}

/// Derive a stable crate label for an `outcomes.json` path relative to `root`.
///
/// For `root/<crate>/outcomes.json` or `root/<crate>/mutants.out/outcomes.json`
/// this returns `<crate>`. A top-level `root/outcomes.json` yields `workspace`.
fn outcomes_crate_label(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .filter(|first| *first != "outcomes.json")
        .unwrap_or("workspace")
        .to_string()
}

/// Extract the per-crate counts cargo-mutants records in `outcomes.json`.
///
/// cargo-mutants writes a top-level `outcomes` array where each entry has a
/// `summary` string ("`CaughtMutant`", "`MissedMutant`", "Timeout", ...) and a
/// `total_phases.*` / `phase_results` timing. We tolerate schema drift by
/// counting on the `summary` field and summing any numeric `*_time` we find.
fn summarize_outcomes(name: &str, parsed: &serde_json::Value) -> serde_json::Value {
    let mut killed = 0u64;
    let mut survivors = 0u64;
    let mut timeouts = 0u64;
    let mut other = 0u64;
    let mut total = 0u64;

    if let Some(outcomes) = parsed.get("outcomes").and_then(|value| value.as_array()) {
        for outcome in outcomes {
            total += 1;
            let summary = outcome
                .get("summary")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            match summary {
                "CaughtMutant" => killed += 1,
                "MissedMutant" => survivors += 1,
                "Timeout" => timeouts += 1,
                _ => other += 1,
            }
        }
    }

    let runtime_secs = parsed
        .get("elapsed_secs")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);

    serde_json::json!({
        "crate": name,
        "runtimeSecs": runtime_secs,
        "total": total,
        "killed": killed,
        "survivors": survivors,
        "timeouts": timeouts,
        "other": other,
    })
}

/// Validate the namespaced endpoint catalog's intrinsic invariants. Mirrors
/// the `*-schema-check` command structure: pure, offline, and failing with a
/// structured list of offenders.
///
/// Checks, for every catalog entry:
///   1. **Route grammar** — the route obeys the catalog grammar (lowercase
///      `[a-z0-9_]` segments separated by `/`).
///   2. **Schema presence** — both the params and model schema closures produce
///      a non-trivial JSON schema (a fetch route with empty schemas is a wiring
///      bug that would surface as an empty `OpenAPI`/SDK shape downstream).
///   3. **Fetch routes** declare at least one candidate; **Compute routes**
///      carry none.
///
/// Candidate dispatchability (every `Fetch` candidate having an ingest
/// binding) is deliberately NOT checked here: it needs `tdw-service-api` built
/// with `all-http-providers`, and an xtask dependency on that would
/// feature-unify every HTTP provider crate into default workspace builds and
/// lints. It lives instead as the feature-gated test
/// `catalog_candidates_all_dispatchable_under_full_providers` in
/// `tdw-service-api`, run by CI via
/// `cargo test -p tdw-service-api --features all-http-providers`.
fn catalog_check() -> Result<(), String> {
    let mut offenders = Vec::new();
    let mut routes = 0usize;
    let mut candidates_seen = 0usize;

    for entry in tdw_endpoint_catalog::catalog() {
        routes += 1;
        let route = entry.route;

        if !tdw_endpoint_catalog::is_valid_route(route) {
            offenders.push(format!("{route}: route violates the catalog grammar"));
        }

        // Schema presence: both closures must yield a non-empty object schema.
        if schema_is_trivial(&(entry.params_schema)()) {
            offenders.push(format!("{route}: params schema is empty/trivial"));
        }
        if schema_is_trivial(&(entry.model)()) {
            offenders.push(format!("{route}: model schema is empty/trivial"));
        }

        match entry.kind {
            tdw_endpoint_catalog::EndpointKind::Fetch => {
                if entry.candidates.is_empty() {
                    offenders.push(format!("{route}: fetch route has no candidates"));
                }
                candidates_seen += entry.candidates.len();
            }
            tdw_endpoint_catalog::EndpointKind::Compute => {
                if !entry.candidates.is_empty() {
                    offenders.push(format!(
                        "{route}: compute route must not declare provider candidates"
                    ));
                }
            }
        }
    }

    if offenders.is_empty() {
        println!(
            "catalog-check passed: {routes} routes, {candidates_seen} candidates; \
             dispatchability covered by the all-http-providers test in tdw-service-api"
        );
        Ok(())
    } else {
        Err(format!("catalog-check failed:\n{}", offenders.join("\n")))
    }
}

/// Whether a `schemars::Schema` is empty/trivial (no object body to describe a
/// request or record). A real params/model schema serializes to a JSON object
/// with at least one descriptive key (`properties`, `$ref`, `type`, …).
fn schema_is_trivial(schema: &schemars::Schema) -> bool {
    !matches!(
        serde_json::to_value(schema),
        Ok(serde_json::Value::Object(map)) if !map.is_empty()
    )
}

fn clean_room_audit() -> Result<(), String> {
    let mut offenders = Vec::new();
    for path in source_files()? {
        let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        for (index, line) in content.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            let forbidden = [
                format!("{}{}", "finx", "-"),
                format!("{}{}", "tesser", "-"),
                format!("{}{}{}", "tdw-provider", "-", "openbb"),
            ];
            if forbidden.iter().any(|needle| lower.contains(needle)) {
                offenders.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
            }
        }
    }

    if offenders.is_empty() {
        println!("clean-room audit passed");
        Ok(())
    } else {
        Err(format!(
            "clean-room audit failed:\n{}",
            offenders.join("\n")
        ))
    }
}

fn crate_readiness_check() -> Result<(), String> {
    let workspace_crates = workspace_package_names()?;
    let matrix_crates = matrix_crate_names("docs/quality/crate-readiness/matrix.md")?;

    let mut missing_matrix_rows = Vec::new();
    let mut missing_worksheets = Vec::new();
    for name in &workspace_crates {
        if !matrix_crates.contains(name) {
            missing_matrix_rows.push(name.clone());
        }
        let worksheet = PathBuf::from("docs/quality/crate-readiness").join(format!("{name}.md"));
        if !worksheet.is_file() {
            missing_worksheets.push(name.clone());
        }
    }

    let stale_matrix_rows: Vec<_> = matrix_crates
        .iter()
        .filter(|name| !workspace_crates.contains(*name))
        .cloned()
        .collect();

    if missing_matrix_rows.is_empty()
        && missing_worksheets.is_empty()
        && stale_matrix_rows.is_empty()
    {
        println!(
            "crate-readiness-check passed: {} workspace crates covered",
            workspace_crates.len()
        );
        return Ok(());
    }

    let mut sections = Vec::new();
    if !missing_matrix_rows.is_empty() {
        sections.push(format!(
            "missing matrix rows: {}",
            missing_matrix_rows.join(", ")
        ));
    }
    if !missing_worksheets.is_empty() {
        sections.push(format!(
            "missing worksheets: {}",
            missing_worksheets.join(", ")
        ));
    }
    if !stale_matrix_rows.is_empty() {
        sections.push(format!(
            "stale matrix rows: {}",
            stale_matrix_rows.join(", ")
        ));
    }

    Err(format!(
        "crate-readiness-check failed; refresh docs/quality/crate-readiness:\n{}",
        sections.join("\n")
    ))
}

fn workspace_package_names() -> Result<Vec<String>, String> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|error| format!("failed to spawn cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata exited with {}", output.status));
    }
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    let mut names: Vec<String> = parsed
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "cargo metadata output missing packages array".to_string())?
        .iter()
        .filter(|package| package.get("source").is_none_or(serde_json::Value::is_null))
        .filter_map(|package| package.get("name").and_then(serde_json::Value::as_str))
        .map(ToString::to_string)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

fn matrix_crate_names(path: &str) -> Result<Vec<String>, String> {
    let content = fs::read_to_string(path).map_err(|error| {
        format!("crate-readiness matrix missing or unreadable at {path}: {error}")
    })?;
    let mut names: Vec<String> = content
        .lines()
        .filter_map(|line| {
            line.strip_prefix("| [")
                .and_then(|rest| rest.split_once(']'))
                .map(|(name, _)| name.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// TEST-POLICY-005: run the stable pre-release fuzz-smoke + loom evidence in one
/// command. This is a manual release-candidate step, not a phase-exit gate.
///
/// It shells out to two stable suites (deterministic, stable toolchain):
///   1. the corpus-replay fuzz harnesses (`tests/fuzz_replay.rs`) across the six
///      parser/wire-format surfaces, run as a normal `cargo test`;
///   2. the `tdw-app-server` loom relay model, run with `RUSTFLAGS=--cfg loom`
///      scoped to that single child process only (never set globally).
///
/// Deep, coverage-guided fuzzing stays the nightly `fuzz-smoke` CI job and the
/// manual `cargo +nightly fuzz run <target>` path; this command only proves the
/// stable smoke evidence is green before a release cut. Returns `Err` if either
/// suite fails so release readiness cannot claim fuzz/loom evidence without it.
fn prerelease_check() -> Result<(), String> {
    println!("prerelease-check: running crate-readiness sync + stable fuzz-smoke + loom evidence");

    println!("prerelease-check: [1/3] crate-readiness coverage sync");
    let crate_readiness_ok = match crate_readiness_check() {
        Ok(()) => true,
        Err(error) => {
            eprintln!("{error}");
            false
        }
    };

    println!("prerelease-check: [2/3] stable fuzz corpus-replay harnesses");
    let fuzz_ok = run_check(std::process::Command::new("cargo").args([
        "test",
        "-p",
        "tdw-protocol",
        "-p",
        "tdw-config",
        "-p",
        "tdw-mcp",
        "-p",
        "tdw-app-client",
        "-p",
        "tdw-exec",
        "--test",
        "fuzz_replay",
    ]));

    println!("prerelease-check: [3/3] stable loom relay model (RUSTFLAGS=--cfg loom)");
    let loom_ok = run_check(
        std::process::Command::new("cargo")
            .args(["test", "-p", "tdw-app-server", "--test", "loom_relay"])
            .env("RUSTFLAGS", "--cfg loom"),
    );

    println!("prerelease-check: summary");
    println!(
        "  crate-readiness coverage: {}",
        pass_label(crate_readiness_ok)
    );
    println!("  fuzz-smoke (corpus replay): {}", pass_label(fuzz_ok));
    println!("  loom relay model:           {}", pass_label(loom_ok));
    println!(
        "  deep fuzzing: nightly `fuzz-smoke` job / `cargo +nightly fuzz run <target>` (not run here)"
    );

    if crate_readiness_ok && fuzz_ok && loom_ok {
        println!("prerelease-check: PASS");
        Ok(())
    } else {
        Err("prerelease-check: FAIL (see suite output above)".to_string())
    }
}

/// Run a child command inheriting stdio and report whether it exited `0`.
fn run_check(command: &mut std::process::Command) -> bool {
    match command.status() {
        Ok(status) => status.success(),
        Err(error) => {
            eprintln!("prerelease-check: failed to spawn command: {error}");
            false
        }
    }
}

const fn pass_label(ok: bool) -> &'static str {
    if ok { "PASS" } else { "FAIL" }
}

fn source_files() -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_files(Path::new("Cargo.toml"), &mut files)?;
    collect_files(Path::new("crates"), &mut files)?;
    Ok(files)
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("rs" | "toml")
        ) || path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
        {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }

    if !path.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        collect_files(&entry.path(), files)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn quality_gate_covers_layer_f_required_commands() {
        let ids = quality_gates()
            .iter()
            .map(|gate| gate.id)
            .collect::<HashSet<_>>();
        for required in [
            "fmt",
            "lint",
            "unit",
            "integration",
            "property",
            "e2e-subset",
            "adversarial",
            "coverage",
            "schema-sync",
            "event-schema-check",
            "perf",
            "dependency-audit",
            "clean-room-audit",
            "windows-release",
            "mutation-smoke",
            "mutation-summary",
        ] {
            assert!(ids.contains(required), "missing gate {required}");
        }
    }

    #[test]
    fn quality_gate_json_is_stable_and_enforces_blockers() {
        let content = quality_gate_json().expect("quality gate json should serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("quality gate json should parse");
        assert_eq!(parsed["policy"]["failedGateRequiresBlockerEvidence"], true);
        assert_eq!(parsed["gates"].as_array().expect("gates array").len(), 18);
    }

    #[test]
    fn crate_for_path_maps_only_crate_sources() {
        assert_eq!(
            crate_for_path("crates/tdw-protocol/src/lib.rs"),
            Some("tdw-protocol".to_string())
        );
        assert_eq!(
            crate_for_path("crates\\tdw-worker\\Cargo.toml"),
            Some("tdw-worker".to_string())
        );
        assert_eq!(crate_for_path("xtask/src/main.rs"), None);
        assert_eq!(crate_for_path("docs/quality/test-policy-backlog.md"), None);
        assert_eq!(crate_for_path("crates/"), None);
    }

    #[test]
    fn summarize_outcomes_classifies_cargo_mutants_summaries() {
        let parsed: serde_json::Value = serde_json::json!({
            "elapsed_secs": 12.5,
            "outcomes": [
                { "summary": "CaughtMutant" },
                { "summary": "CaughtMutant" },
                { "summary": "MissedMutant" },
                { "summary": "Timeout" },
                { "summary": "Unviable" },
            ]
        });
        let summary = summarize_outcomes("tdw-core", &parsed);
        assert_eq!(summary["crate"], "tdw-core");
        assert_eq!(summary["runtimeSecs"], 12.5);
        assert_eq!(summary["total"], 5);
        assert_eq!(summary["killed"], 2);
        assert_eq!(summary["survivors"], 1);
        assert_eq!(summary["timeouts"], 1);
        assert_eq!(summary["other"], 1);
    }

    #[test]
    fn summarize_outcomes_tolerates_missing_fields() {
        let parsed = serde_json::json!({});
        let summary = summarize_outcomes("tdw-mcp", &parsed);
        assert_eq!(summary["total"], 0);
        assert_eq!(summary["killed"], 0);
        assert_eq!(summary["runtimeSecs"], 0.0);
    }

    #[test]
    fn mutation_baseline_includes_required_scope() {
        for required in [
            "tdw-core",
            "tdw-protocol",
            "tdw-app-client",
            "tdw-mcp",
            "tdw-worker",
        ] {
            assert!(
                MUTATION_BASELINE_CRATES.contains(&required),
                "missing baseline crate {required}"
            );
        }
    }
}
