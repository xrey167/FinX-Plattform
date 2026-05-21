#![forbid(unsafe_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineJob {
    pub name: &'static str,
    pub runner: &'static str,
    pub args: &'static str,
    pub depends_on: &'static [&'static str],
}

pub fn market_data_dbt_jobs() -> Vec<PipelineJob> {
    vec![
        PipelineJob {
            name: "dbt_bronze_market_data",
            runner: "dbt",
            args: "run --select tag:layer:bronze tag:domain:market_data",
            depends_on: &[],
        },
        PipelineJob {
            name: "dbt_silver_market_data",
            runner: "dbt",
            args: "run --select tag:layer:silver tag:domain:market_data",
            depends_on: &["dbt_bronze_market_data"],
        },
        PipelineJob {
            name: "dbt_gold_market_data",
            runner: "dbt",
            args: "run --select tag:layer:gold tag:domain:market_data",
            depends_on: &["dbt_silver_market_data"],
        },
        PipelineJob {
            name: "dbt_test_market_data",
            runner: "dbt",
            args: "test --select tag:domain:market_data",
            depends_on: &["dbt_gold_market_data"],
        },
    ]
}

pub fn can_enqueue(job: &PipelineJob, completed_jobs: &[&str]) -> bool {
    job.depends_on
        .iter()
        .all(|dependency| completed_jobs.contains(dependency))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silver_waits_for_bronze() {
        let jobs = market_data_dbt_jobs();
        let silver = jobs
            .iter()
            .find(|job| job.name == "dbt_silver_market_data")
            .unwrap_or_else(|| panic!("silver job should exist"));

        assert!(!can_enqueue(silver, &[]));
        assert!(can_enqueue(silver, &["dbt_bronze_market_data"]));
    }
}
