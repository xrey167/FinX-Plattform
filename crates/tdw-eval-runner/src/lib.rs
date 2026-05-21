#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_agent::{EvalMetric, EvalRunRequest};
use tdw_agent_store::{AgentStore, StoredEvalRun};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalRunOutcome {
    pub run_id: String,
    pub status: String,
    pub metrics: Vec<EvalMetric>,
}

pub struct EvalRunner;

impl EvalRunner {
    pub fn run(request: EvalRunRequest, store: &mut AgentStore) -> EvalRunOutcome {
        let case_count = request.cases.len() as f64;
        let has_agent_card = store.agent(&request.agent_id).is_some();
        let coverage = if case_count > 0.0 && has_agent_card {
            1.0
        } else {
            0.0
        };
        let metrics = vec![
            EvalMetric {
                metric_name: "case_count".to_string(),
                metric_value: case_count,
            },
            EvalMetric {
                metric_name: "agent_card_coverage".to_string(),
                metric_value: coverage,
            },
        ];
        let status = if coverage > 0.0 { "success" } else { "failed" }.to_string();
        let outcome = EvalRunOutcome {
            run_id: request.run_id.clone(),
            status: status.clone(),
            metrics: metrics.clone(),
        };
        store.record_eval_run(StoredEvalRun {
            request,
            metrics,
            status,
        });
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_agent::{EvalCase, EvalRunRequest, sample_agent_card};
    use tdw_agent_store::AgentStore;

    #[test]
    fn runs_eval_and_persists_metrics() {
        let mut store = AgentStore::new();
        store.upsert_agent(sample_agent_card());
        let outcome = EvalRunner::run(
            EvalRunRequest {
                run_id: "eval-1".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden-market-notes".to_string(),
                cases: vec![EvalCase {
                    case_id: "case-1".to_string(),
                    prompt: "Summarize AAPL".to_string(),
                    expected_refs: Vec::new(),
                }],
            },
            &mut store,
        );

        assert_eq!(outcome.status, "success");
        assert_eq!(
            store.eval_run("eval-1").map(|run| run.metrics.len()),
            Some(2)
        );
    }
}
