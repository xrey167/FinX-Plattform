#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tdw_agent::{
    AgentCard, EvalMetric, EvalRunRequest, Gotcha, StorageMapping, WorkflowDefinition,
    agent_storage_mappings,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredEvalRun {
    pub request: EvalRunRequest,
    pub metrics: Vec<EvalMetric>,
    pub status: String,
}

#[derive(Clone, Debug, Default)]
pub struct AgentStore {
    agents: BTreeMap<String, AgentCard>,
    gotchas: BTreeMap<String, Gotcha>,
    workflows: BTreeMap<String, WorkflowDefinition>,
    eval_runs: BTreeMap<String, StoredEvalRun>,
}

impl AgentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_agent(&mut self, card: AgentCard) {
        self.agents.insert(card.agent_id.clone(), card);
    }

    pub fn upsert_gotcha(&mut self, gotcha: Gotcha) {
        self.gotchas.insert(gotcha.gotcha_id.clone(), gotcha);
    }

    pub fn upsert_workflow(&mut self, workflow: WorkflowDefinition) {
        self.workflows
            .insert(workflow.workflow_id.clone(), workflow);
    }

    pub fn record_eval_run(&mut self, run: StoredEvalRun) {
        self.eval_runs.insert(run.request.run_id.clone(), run);
    }

    pub fn agent(&self, agent_id: &str) -> Option<&AgentCard> {
        self.agents.get(agent_id)
    }

    pub fn gotcha(&self, gotcha_id: &str) -> Option<&Gotcha> {
        self.gotchas.get(gotcha_id)
    }

    pub fn workflow(&self, workflow_id: &str) -> Option<&WorkflowDefinition> {
        self.workflows.get(workflow_id)
    }

    pub fn eval_run(&self, run_id: &str) -> Option<&StoredEvalRun> {
        self.eval_runs.get(run_id)
    }

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
}
