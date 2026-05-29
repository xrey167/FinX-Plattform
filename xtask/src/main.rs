use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".to_string());
    let result = match command.as_str() {
        "bench" => bench(),
        "bench-compare" => bench_compare(args.next()),
        "quality-gate" => quality_gate(args.next()),
        "ddl-export" => ddl_export(args.next()),
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
            Some("changed") => mutation_changed(args.next()),
            Some("report") => mutation_report(args.next()),
            _ => help(),
        },
        "clean-room-audit" => clean_room_audit(),
        "prerelease-check" => prerelease_check(),
        _ => help(),
    };

    if let Err(error) = result {
        eprintln!("xtask error: {error}");
        std::process::exit(1);
    }
}

fn help() -> Result<(), String> {
    println!(
        "xtask commands: bench | bench-compare <baseline> | quality-gate <write|check> | ddl-export <postgres|clickhouse> | migrate <up|down|status> | schema-sync | events schema-check | protocol schema-check | config schema-check | mutation <changed [--run]|report [out-dir]> | clean-room-audit | prerelease-check"
    );
    Ok(())
}

fn bench() -> Result<(), String> {
    fs::create_dir_all("docs").map_err(|error| error.to_string())?;
    fs::write(
        "docs/perf-history.json",
        "{\n  \"bootstrap\": true,\n  \"workloads\": []\n}\n",
    )
    .map_err(|error| error.to_string())?;
    println!("bench scaffold wrote docs/perf-history.json");
    Ok(())
}

fn bench_compare(baseline: Option<String>) -> Result<(), String> {
    println!(
        "bench comparison scaffold; baseline={}",
        baseline.unwrap_or_else(|| "none".to_string())
    );
    Ok(())
}

fn quality_gate(mode: Option<String>) -> Result<(), String> {
    match mode.as_deref().unwrap_or("write") {
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

fn ddl_export(target: Option<String>) -> Result<(), String> {
    let target = match target.as_deref() {
        Some("postgres") => tdw_sql_codegen::SqlTarget::Postgres,
        Some("clickhouse") => tdw_sql_codegen::SqlTarget::ClickHouse,
        Some(other) => return Err(format!("unknown ddl target: {other}")),
        None => tdw_sql_codegen::SqlTarget::Postgres,
    };
    print!("{}", tdw_sql_codegen::export_domain_ddl(target));
    Ok(())
}

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

fn migrate_down() -> Result<(), String> {
    println!("offline migrate down plan: no destructive migration is run by xtask scaffold");
    Ok(())
}

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
        tdw_event::event_schema_bundle(),
    )?;
    println!("events schema-check wrote {count} event schemas to docs/schemas/event");
    Ok(())
}

fn protocol_schema_check() -> Result<(), String> {
    let count = write_schema_bundle(
        Path::new("docs/schemas/protocol"),
        tdw_protocol::schema_bundle(),
    )?;
    println!("protocol schema-check wrote {count} protocol schemas to docs/schemas/protocol");
    Ok(())
}

fn config_schema_check() -> Result<(), String> {
    let count = write_schema_bundle(
        Path::new("docs/schemas/config"),
        tdw_config::schema_bundle(),
    )?;
    println!("config schema-check wrote {count} config schemas to docs/schemas/config");
    Ok(())
}

fn write_schema_bundle(
    schema_dir: &Path,
    bundle: std::collections::BTreeMap<&'static str, serde_json::Value>,
) -> Result<usize, String> {
    fs::create_dir_all(schema_dir).map_err(|error| error.to_string())?;
    for (name, schema) in &bundle {
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
fn mutation_changed(flag: Option<String>) -> Result<(), String> {
    let run = matches!(flag.as_deref(), Some("--run" | "run"));
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
/// `summary` string ("CaughtMutant", "MissedMutant", "Timeout", ...) and a
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
    println!("prerelease-check: running stable fuzz-smoke + loom evidence");

    println!("prerelease-check: [1/2] stable fuzz corpus-replay harnesses");
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

    println!("prerelease-check: [2/2] stable loom relay model (RUSTFLAGS=--cfg loom)");
    let loom_ok = run_check(
        std::process::Command::new("cargo")
            .args(["test", "-p", "tdw-app-server", "--test", "loom_relay"])
            .env("RUSTFLAGS", "--cfg loom"),
    );

    println!("prerelease-check: summary");
    println!("  fuzz-smoke (corpus replay): {}", pass_label(fuzz_ok));
    println!("  loom relay model:           {}", pass_label(loom_ok));
    println!(
        "  deep fuzzing: nightly `fuzz-smoke` job / `cargo +nightly fuzz run <target>` (not run here)"
    );

    if fuzz_ok && loom_ok {
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

fn pass_label(ok: bool) -> &'static str {
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
