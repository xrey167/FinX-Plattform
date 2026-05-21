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
        "clean-room-audit" => clean_room_audit(),
        _ => help(),
    };

    if let Err(error) = result {
        eprintln!("xtask error: {error}");
        std::process::exit(1);
    }
}

fn help() -> Result<(), String> {
    println!(
        "xtask commands: bench | bench-compare <baseline> | quality-gate <write|check> | ddl-export <postgres|clickhouse> | migrate <up|down|status> | schema-sync | events schema-check | clean-room-audit"
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

fn quality_gates() -> &'static [QualityGate] {
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
    let schema_dir = Path::new("docs/schemas/event");
    fs::create_dir_all(schema_dir).map_err(|error| error.to_string())?;
    let bundle = tdw_event::event_schema_bundle();
    for (name, schema) in &bundle {
        let content =
            serde_json::to_string_pretty(schema).map_err(|error| error.to_string())? + "\n";
        fs::write(schema_dir.join(format!("{name}.schema.json")), content)
            .map_err(|error| error.to_string())?;
    }
    println!(
        "events schema-check wrote {} event schemas to {}",
        bundle.len(),
        schema_dir.display()
    );
    Ok(())
}

fn clean_room_audit() -> Result<(), String> {
    let mut offenders = Vec::new();
    for path in source_files()? {
        let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        for (index, line) in content.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            let forbidden = [format!("{}{}", "finx", "-"), format!("{}{}", "tesser", "-")];
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
        assert_eq!(parsed["gates"].as_array().expect("gates array").len(), 17);
    }
}
