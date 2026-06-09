#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
use std::collections::BTreeSet;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, PipelineValidationError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineJob {
    pub name: &'static str,
    pub runner: &'static str,
    pub args: &'static str,
    pub depends_on: &'static [&'static str],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PipelineValidationError {
    #[error("pipeline must include at least one job")]
    EmptyPipeline,
    #[error("pipeline job name must not be empty")]
    EmptyJobName,
    #[error("pipeline job {name} runner must not be empty")]
    EmptyRunner { name: &'static str },
    #[error("pipeline job {name} args must not be empty")]
    EmptyArgs { name: &'static str },
    #[error("duplicate pipeline job: {name}")]
    DuplicateJob { name: &'static str },
    #[error("pipeline job {name} depends on itself")]
    SelfDependency { name: &'static str },
    #[error("pipeline job {name} has unknown dependency {dependency}")]
    UnknownDependency {
        name: &'static str,
        dependency: &'static str,
    },
}

#[must_use]
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

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_jobs(jobs: &[PipelineJob]) -> Result<()> {
    if jobs.is_empty() {
        return Err(PipelineValidationError::EmptyPipeline);
    }

    let mut names = BTreeSet::new();
    for job in jobs {
        if job.name.trim().is_empty() {
            return Err(PipelineValidationError::EmptyJobName);
        }
        if job.runner.trim().is_empty() {
            return Err(PipelineValidationError::EmptyRunner { name: job.name });
        }
        if job.args.trim().is_empty() {
            return Err(PipelineValidationError::EmptyArgs { name: job.name });
        }
        if !names.insert(job.name) {
            return Err(PipelineValidationError::DuplicateJob { name: job.name });
        }
    }

    for job in jobs {
        for dependency in job.depends_on {
            if *dependency == job.name {
                return Err(PipelineValidationError::SelfDependency { name: job.name });
            }
            if !names.contains(dependency) {
                return Err(PipelineValidationError::UnknownDependency {
                    name: job.name,
                    dependency,
                });
            }
        }
    }

    Ok(())
}

#[must_use]
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
        assert!(validate_jobs(&jobs).is_ok());
    }

    #[test]
    fn rejects_duplicate_and_unknown_pipeline_dependencies() {
        let duplicate = vec![
            PipelineJob {
                name: "job",
                runner: "dbt",
                args: "run",
                depends_on: &[],
            },
            PipelineJob {
                name: "job",
                runner: "dbt",
                args: "run",
                depends_on: &[],
            },
        ];
        let unknown = vec![PipelineJob {
            name: "silver",
            runner: "dbt",
            args: "run",
            depends_on: &["bronze"],
        }];

        assert!(validate_jobs(&duplicate).is_err());
        assert!(validate_jobs(&unknown).is_err());
        assert_eq!(
            validate_jobs(&[PipelineJob {
                name: "job",
                runner: "dbt",
                args: "",
                depends_on: &[],
            }]),
            Err(PipelineValidationError::EmptyArgs { name: "job" })
        );
    }
}
