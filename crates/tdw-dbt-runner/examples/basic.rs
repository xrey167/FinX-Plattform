//! Offline `tdw-dbt-runner` example: build a validated `dbt run` command and
//! parse a sample `run_results.json` body into reportable rows.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-dbt-runner --example tdw-dbt-runner-basic
//! ```

use tdw_dbt_runner::{DbtCommand, parse_run_results, run_step_rows};

fn main() {
    // Meaningful operation: build a validated, shell-free dbt command.
    let command = DbtCommand::build_run("dbt/finx_finance", "tag:layer:bronze")
        .expect("dbt command should be valid");
    println!(
        "command: dbt {} (project={}, profiles={})",
        command.args.join(" "),
        command.project_dir,
        command.profiles_dir
    );

    // A selector with control characters is rejected.
    println!(
        "control-char selector rejected: {}",
        DbtCommand::build_run("dbt/finx_finance", "\n").is_err()
    );

    // Parse a dbt run_results.json body and flatten it for reporting.
    let result = parse_run_results(
        r#"{"results":[{"unique_id":"model.proj.bronze_ohlcv","status":"success","execution_time":0.12}]}"#,
    )
    .expect("run_results should parse");
    for (node_id, status, seconds) in run_step_rows(&result) {
        println!("  {node_id}: {status} ({seconds}s)");
    }
}
