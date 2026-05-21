#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

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

impl DbtCommand {
    pub fn build_run(project_dir: impl Into<String>, selector: impl Into<String>) -> Self {
        Self {
            project_dir: project_dir.into(),
            profiles_dir: "dbt/finx_finance".to_string(),
            args: vec!["run".to_string(), "--select".to_string(), selector.into()],
        }
    }
}

pub fn parse_run_results(json: &str) -> Result<DbtRunResult, serde_json::Error> {
    serde_json::from_str(json)
}

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
}
