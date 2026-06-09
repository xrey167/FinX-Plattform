#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, DbtCommandError>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DbtRunResult {
    pub results: Vec<DbtNodeResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DbtNodeResult {
    #[serde(rename = "unique_id")]
    pub node_id: String,
    pub status: String,
    pub execution_time: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbtCommand {
    pub project_dir: String,
    pub profiles_dir: String,
    pub args: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DbtCommandError {
    #[error("dbt project dir must not be empty")]
    EmptyProjectDir,
    #[error("dbt profiles dir must not be empty")]
    EmptyProfilesDir,
    #[error("dbt selector must not be empty")]
    EmptySelector,
    #[error("dbt selector must not contain control characters")]
    SelectorControlCharacters,
}

impl DbtCommand {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn build_run(project_dir: impl Into<String>, selector: impl Into<String>) -> Result<Self> {
        Self::build_run_with_profiles(project_dir, "dbt/finx_finance", selector)
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn build_run_with_profiles(
        project_dir: impl Into<String>,
        profiles_dir: impl Into<String>,
        selector: impl Into<String>,
    ) -> Result<Self> {
        let project_dir = project_dir.into();
        let profiles_dir = profiles_dir.into();
        let selector = selector.into();

        validate_component(&project_dir, DbtCommandError::EmptyProjectDir)?;
        validate_component(&profiles_dir, DbtCommandError::EmptyProfilesDir)?;
        validate_component(&selector, DbtCommandError::EmptySelector)?;
        if selector.chars().any(char::is_control) {
            return Err(DbtCommandError::SelectorControlCharacters);
        }

        Ok(Self {
            project_dir,
            profiles_dir,
            args: vec!["run".to_string(), "--select".to_string(), selector],
        })
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn parse_run_results(json: &str) -> std::result::Result<DbtRunResult, serde_json::Error> {
    serde_json::from_str(json)
}

#[must_use]
pub fn run_step_rows(result: &DbtRunResult) -> Vec<(String, String, f64)> {
    result
        .results
        .iter()
        .map(|node| {
            (
                node.node_id.clone(),
                node.status.clone(),
                node.execution_time,
            )
        })
        .collect()
}

fn validate_component(value: &str, error: DbtCommandError) -> Result<()> {
    if value.trim().is_empty() {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dbt_run_results_fixture() {
        let result = parse_run_results(
            r#"{"results":[{"unique_id":"model.finx_finance.bronze_ohlcv","status":"success","execution_time":0.12}]}"#,
        )
        .unwrap_or_else(|error| panic!("run_results fixture should parse: {error}"));

        let rows = run_step_rows(&result);
        assert_eq!(rows[0].0, "model.finx_finance.bronze_ohlcv");
        assert_eq!(rows[0].1, "success");
    }

    #[test]
    fn builds_validated_dbt_run_command() {
        let command = DbtCommand::build_run("dbt/finx_finance", "tag:layer:bronze")
            .unwrap_or_else(|error| panic!("dbt command should be valid: {error}"));

        assert_eq!(command.project_dir, "dbt/finx_finance");
        assert_eq!(command.profiles_dir, "dbt/finx_finance");
        assert_eq!(
            command.args,
            vec![
                "run".to_string(),
                "--select".to_string(),
                "tag:layer:bronze".to_string(),
            ]
        );
        assert!(DbtCommand::build_run("", "tag:layer:bronze").is_err());
        assert!(DbtCommand::build_run("dbt/finx_finance", "\n").is_err());
    }
}
