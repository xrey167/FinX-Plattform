//! Offline `tdw-pipeline` example: validate the built-in market-data dbt DAG and
//! walk a simple scheduler loop using `can_enqueue`.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-pipeline --example basic
//! ```

use tdw_pipeline::{can_enqueue, market_data_dbt_jobs, validate_jobs};

fn main() {
    let jobs = market_data_dbt_jobs();

    // Meaningful operation: prove the DAG is structurally well-formed.
    validate_jobs(&jobs).expect("pipeline should validate");
    println!("validated {} jobs", jobs.len());

    // Drive a tiny topological scheduler: repeatedly enqueue ready jobs.
    let mut completed: Vec<&str> = Vec::new();
    while completed.len() < jobs.len() {
        let next = jobs
            .iter()
            .find(|job| !completed.contains(&job.name) && can_enqueue(job, &completed))
            .expect("a runnable job should exist while work remains");
        println!("run: {} ({})", next.name, next.args);
        completed.push(next.name);
    }

    println!("completed order: {completed:?}");
}
