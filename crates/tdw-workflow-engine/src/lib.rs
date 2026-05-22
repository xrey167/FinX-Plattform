#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_agent::{AgentContractError, WorkflowDefinition, validate_workflow_contract};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub workflow_id: String,
    pub ordered_node_ids: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct WorkflowEngine;

impl WorkflowEngine {
    pub fn compile(workflow: &WorkflowDefinition) -> Result<ExecutionPlan, AgentContractError> {
        Ok(ExecutionPlan {
            workflow_id: workflow.workflow_id.clone(),
            ordered_node_ids: validate_workflow_contract(workflow)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_agent::{WorkflowDefinition, WorkflowEdge, WorkflowNode};

    #[test]
    fn compiles_dag_to_execution_plan() {
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

        let plan = WorkflowEngine::compile(&workflow)
            .unwrap_or_else(|error| panic!("workflow should compile: {error}"));
        assert_eq!(plan.ordered_node_ids, vec!["retrieve", "draft"]);
    }

    #[test]
    fn compile_rejects_invalid_workflow_identifiers() {
        let workflow = WorkflowDefinition {
            workflow_id: "../research".to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
        };

        assert!(matches!(
            WorkflowEngine::compile(&workflow),
            Err(AgentContractError::InvalidIdentifier("workflow_id", _))
        ));
    }
}
