#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_agent::{WorkflowDefinition, WorkflowValidationError};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub workflow_id: String,
    pub ordered_node_ids: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct WorkflowEngine;

impl WorkflowEngine {
    pub fn compile(
        workflow: &WorkflowDefinition,
    ) -> Result<ExecutionPlan, WorkflowValidationError> {
        Ok(ExecutionPlan {
            workflow_id: workflow.workflow_id.clone(),
            ordered_node_ids: workflow.validate_dag()?,
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
}
