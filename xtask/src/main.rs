use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".to_string());
    let result = match command.as_str() {
        "bench" => bench(),
        "bench-compare" => bench_compare(args.next()),
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
        "xtask commands: bench | bench-compare <baseline> | schema-sync | events schema-check | clean-room-audit"
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

fn schema_sync() -> Result<(), String> {
    let schema_dir = Path::new("docs/schemas");
    if !schema_dir.exists() {
        println!("schema-sync bootstrap mode: docs/schemas does not exist yet");
        return Ok(());
    }
    println!("schema-sync bootstrap mode: schema directory present");
    Ok(())
}

fn events_schema_check() -> Result<(), String> {
    println!("events schema-check bootstrap mode: no event schemas are emitted yet");
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
