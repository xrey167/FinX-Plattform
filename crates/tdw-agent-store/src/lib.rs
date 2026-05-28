#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tdw_agent::{
    AgentCard, EvalMetric, EvalRunRequest, Gotcha, StorageMapping, WorkflowDefinition,
    agent_storage_mappings, validate_agent_card_contract, validate_workflow_contract,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredEvalRun {
    pub request: EvalRunRequest,
    pub metrics: Vec<EvalMetric>,
    pub status: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    InvalidAgent,
    InvalidWorkflow,
    InvalidEvalRun,
}

#[derive(Clone, Debug, Default)]
pub struct AgentStore {
    agents: BTreeMap<String, AgentCard>,
    gotchas: BTreeMap<String, Gotcha>,
    workflows: BTreeMap<String, WorkflowDefinition>,
    eval_runs: BTreeMap<String, StoredEvalRun>,
}

impl AgentStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_agent(&mut self, card: AgentCard) {
        self.agents.insert(card.agent_id.clone(), card);
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_upsert_agent(&mut self, card: AgentCard) -> Result<(), StoreError> {
        validate_agent_card_contract(&card).map_err(|_| StoreError::InvalidAgent)?;
        self.upsert_agent(card);
        Ok(())
    }

    pub fn upsert_gotcha(&mut self, gotcha: Gotcha) {
        self.gotchas.insert(gotcha.gotcha_id.clone(), gotcha);
    }

    pub fn upsert_workflow(&mut self, workflow: WorkflowDefinition) {
        self.workflows
            .insert(workflow.workflow_id.clone(), workflow);
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_upsert_workflow(&mut self, workflow: WorkflowDefinition) -> Result<(), StoreError> {
        validate_workflow_contract(&workflow).map_err(|_| StoreError::InvalidWorkflow)?;
        self.upsert_workflow(workflow);
        Ok(())
    }

    pub fn record_eval_run(&mut self, run: StoredEvalRun) {
        self.eval_runs.insert(run.request.run_id.clone(), run);
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_record_eval_run(&mut self, run: StoredEvalRun) -> Result<(), StoreError> {
        if run.request.run_id.trim().is_empty()
            || run.request.agent_id.trim().is_empty()
            || run.request.dataset_id.trim().is_empty()
            || run.metrics.iter().any(|metric| {
                metric.metric_name.trim().is_empty() || !metric.metric_value.is_finite()
            })
            || run.status.trim().is_empty()
        {
            return Err(StoreError::InvalidEvalRun);
        }
        self.record_eval_run(run);
        Ok(())
    }

    #[must_use]
    pub fn agent(&self, agent_id: &str) -> Option<&AgentCard> {
        self.agents.get(agent_id)
    }

    #[must_use]
    pub fn gotcha(&self, gotcha_id: &str) -> Option<&Gotcha> {
        self.gotchas.get(gotcha_id)
    }

    #[must_use]
    pub fn workflow(&self, workflow_id: &str) -> Option<&WorkflowDefinition> {
        self.workflows.get(workflow_id)
    }

    #[must_use]
    pub fn eval_run(&self, run_id: &str) -> Option<&StoredEvalRun> {
        self.eval_runs.get(run_id)
    }

    #[must_use]
    pub fn storage_mappings(&self) -> Vec<StorageMapping> {
        agent_storage_mappings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_agent::{
        EvalCase, EvalRunRequest, GotchaSeverity, WorkflowDefinition, WorkflowEdge, WorkflowNode,
        sample_agent_card,
    };

    #[test]
    fn persists_agent_card_gotcha_workflow_and_eval_run() {
        let mut store = AgentStore::new();
        let card = sample_agent_card();
        store.upsert_agent(card.clone());
        assert_eq!(store.agent("market-researcher"), Some(&card));

        let gotcha = Gotcha {
            gotcha_id: "needs-provenance".to_string(),
            title: "Needs provenance".to_string(),
            severity: GotchaSeverity::Warning,
            applies_to: vec!["research.note".to_string()],
            remediation: "Attach content refs.".to_string(),
            source_ref: None,
        };
        store.upsert_gotcha(gotcha.clone());
        assert_eq!(store.gotcha("needs-provenance"), Some(&gotcha));

        let workflow = WorkflowDefinition {
            workflow_id: "research-flow".to_string(),
            nodes: vec![
                WorkflowNode {
                    node_id: "retrieve".to_string(),
                    task: "retrieve".to_string(),
                    skill_id: None,
                },
                WorkflowNode {
                    node_id: "draft".to_string(),
                    task: "draft".to_string(),
                    skill_id: Some("research.note".to_string()),
                },
            ],
            edges: vec![WorkflowEdge {
                from: "retrieve".to_string(),
                to: "draft".to_string(),
            }],
        };
        store.upsert_workflow(workflow.clone());
        assert_eq!(store.workflow("research-flow"), Some(&workflow));

        let eval_request = EvalRunRequest {
            run_id: "eval-1".to_string(),
            agent_id: "market-researcher".to_string(),
            dataset_id: "golden-market-notes".to_string(),
            cases: vec![EvalCase {
                case_id: "case-1".to_string(),
                prompt: "Summarize AAPL".to_string(),
                expected_refs: Vec::new(),
            }],
        };
        let eval_run = StoredEvalRun {
            request: eval_request,
            metrics: Vec::new(),
            status: "success".to_string(),
        };
        store.record_eval_run(eval_run.clone());
        assert_eq!(store.eval_run("eval-1"), Some(&eval_run));

        let mappings = store.storage_mappings();
        assert!(mappings.iter().any(|mapping| mapping.schema == "agents"));
        assert!(mappings.iter().any(|mapping| mapping.schema == "evals"));
    }

    #[test]
    fn checked_store_paths_reject_invalid_agent_workflow_and_eval_run() {
        let mut store = AgentStore::new();
        let mut card = sample_agent_card();
        card.agent_id = "../agent".to_string();
        assert_eq!(store.try_upsert_agent(card), Err(StoreError::InvalidAgent));

        assert_eq!(
            store.try_upsert_workflow(WorkflowDefinition {
                workflow_id: "research-flow".to_string(),
                nodes: Vec::new(),
                edges: vec![WorkflowEdge {
                    from: "missing".to_string(),
                    to: "also-missing".to_string(),
                }],
            }),
            Err(StoreError::InvalidWorkflow)
        );

        assert_eq!(
            store.try_record_eval_run(StoredEvalRun {
                request: EvalRunRequest {
                    run_id: "eval-1".to_string(),
                    agent_id: "market-researcher".to_string(),
                    dataset_id: "golden-market-notes".to_string(),
                    cases: Vec::new(),
                },
                metrics: vec![EvalMetric {
                    metric_name: "score".to_string(),
                    metric_value: f64::NAN,
                }],
                status: "success".to_string(),
            }),
            Err(StoreError::InvalidEvalRun)
        );
    }
}
