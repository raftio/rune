use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::SpecError;

/// A workflow orchestrates multiple agents in a defined DAG pattern via A2A.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub steps: Vec<WorkflowStep>,
    /// Template for composing the final output from step outputs.
    /// Uses `{{ steps.<step_id>.output }}` syntax.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_step: Option<String>,
}

fn default_timeout_ms() -> u64 {
    120_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    /// A2A endpoint for the agent: `local://agent-name` or full URL.
    pub agent_ref: String,
    /// Template for the input to this step. Supports substitution:
    /// - `{{ input }}` — the original workflow input
    /// - `{{ steps.<id>.output }}` — output of a previous step
    #[serde(default = "default_input_template")]
    pub input_template: String,
    /// IDs of steps that must complete before this step can start.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Optional condition expression. If present and evaluates to false,
    /// the step is skipped. Simple equality: `{{ steps.router.output }} == "technical"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Per-step timeout override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

fn default_input_template() -> String {
    "{{ input }}".into()
}

impl WorkflowSpec {
    pub fn load(path: &Path) -> Result<Self, SpecError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| SpecError::Io(path.to_path_buf(), e))?;
        Self::parse(&content, &path.display().to_string())
    }

    pub fn parse(yaml: &str, source_name: &str) -> Result<Self, SpecError> {
        let spec: Self = serde_yaml::from_str(yaml)
            .map_err(|e| SpecError::Parse(source_name.to_string(), e.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    fn validate(&self) -> Result<(), SpecError> {
        if self.steps.is_empty() {
            return Err(SpecError::Parse(
                "workflow.yaml".into(),
                "workflow must have at least one step".into(),
            ));
        }

        let step_ids: std::collections::HashSet<&str> =
            self.steps.iter().map(|s| s.id.as_str()).collect();

        for step in &self.steps {
            if step.id.is_empty() {
                return Err(SpecError::Parse(
                    "workflow.yaml".into(),
                    "step id cannot be empty".into(),
                ));
            }
            for dep in &step.depends_on {
                if !step_ids.contains(dep.as_str()) {
                    return Err(SpecError::Parse(
                        "workflow.yaml".into(),
                        format!(
                            "step '{}' depends on '{}' which does not exist",
                            step.id, dep
                        ),
                    ));
                }
                if dep == &step.id {
                    return Err(SpecError::Parse(
                        "workflow.yaml".into(),
                        format!("step '{}' cannot depend on itself", step.id),
                    ));
                }
            }
        }

        // Detect cycles via topological sort.
        if topological_sort(&self.steps).is_none() {
            return Err(SpecError::Parse(
                "workflow.yaml".into(),
                "workflow steps contain a dependency cycle".into(),
            ));
        }

        Ok(())
    }
}

/// Kahn's algorithm. Returns `None` if the graph has a cycle.
pub fn topological_sort(steps: &[WorkflowStep]) -> Option<Vec<usize>> {
    use std::collections::{HashMap, VecDeque};

    let id_to_idx: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    let n = steps.len();
    let mut in_degree = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (i, step) in steps.iter().enumerate() {
        for dep in &step.depends_on {
            if let Some(&dep_idx) = id_to_idx.get(dep.as_str()) {
                adj[dep_idx].push(i);
                in_degree[i] += 1;
            }
        }
    }

    let mut queue: VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|(_, &d)| d == 0)
        .map(|(i, _)| i)
        .collect();

    let mut order = Vec::with_capacity(n);

    while let Some(node) = queue.pop_front() {
        order.push(node);
        for &next in &adj[node] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push_back(next);
            }
        }
    }

    if order.len() == n {
        Some(order)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_workflow_yaml() -> &'static str {
        r#"
name: pipeline
version: 0.1.0
steps:
  - id: router
    agent_ref: local://router
  - id: worker
    agent_ref: local://worker
    depends_on: [router]
"#
    }

    #[test]
    fn parse_valid_workflow() {
        let spec = WorkflowSpec::parse(simple_workflow_yaml(), "test").unwrap();
        assert_eq!(spec.name, "pipeline");
        assert_eq!(spec.version, "0.1.0");
        assert_eq!(spec.steps.len(), 2);
        assert_eq!(spec.steps[0].id, "router");
        assert_eq!(spec.steps[0].agent_ref, "local://router");
        assert_eq!(spec.steps[0].input_template, "{{ input }}");
        assert!(spec.steps[0].depends_on.is_empty());
        assert_eq!(spec.steps[1].depends_on, vec!["router"]);
    }

    #[test]
    fn defaults_applied() {
        let spec = WorkflowSpec::parse(simple_workflow_yaml(), "test").unwrap();
        assert_eq!(spec.timeout_ms, 120_000);
        assert!(spec.description.is_empty());
        assert!(spec.output_step.is_none());
    }

    #[test]
    fn parse_full_workflow_with_all_fields() {
        let yaml = r#"
name: full
version: 1.0.0
description: Full pipeline
timeout_ms: 60000
output_step: "{{ steps.final.output }}"
steps:
  - id: step1
    agent_ref: local://agent1
    input_template: "{{ input }}"
    timeout_ms: 10000
  - id: step2
    agent_ref: local://agent2
    input_template: "{{ steps.step1.output }}"
    depends_on: [step1]
    condition: "{{ steps.step1.output }} == ok"
  - id: final
    agent_ref: local://final
    depends_on: [step2]
"#;
        let spec = WorkflowSpec::parse(yaml, "test").unwrap();
        assert_eq!(spec.timeout_ms, 60_000);
        assert_eq!(spec.description, "Full pipeline");
        assert_eq!(spec.output_step.as_deref(), Some("{{ steps.final.output }}"));
        assert_eq!(spec.steps[0].timeout_ms, Some(10_000));
        assert_eq!(
            spec.steps[1].condition.as_deref(),
            Some("{{ steps.step1.output }} == ok")
        );
    }

    #[test]
    fn error_empty_steps() {
        let yaml = "name: empty\nversion: 0.1.0\nsteps: []\n";
        let err = WorkflowSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("at least one step"));
    }

    #[test]
    fn error_empty_step_id() {
        let yaml = r#"
name: bad
version: 0.1.0
steps:
  - id: ""
    agent_ref: local://x
"#;
        let err = WorkflowSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("step id cannot be empty"));
    }

    #[test]
    fn error_unknown_dependency() {
        let yaml = r#"
name: bad
version: 0.1.0
steps:
  - id: step1
    agent_ref: local://x
    depends_on: [nonexistent]
"#;
        let err = WorkflowSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn error_self_dependency() {
        let yaml = r#"
name: bad
version: 0.1.0
steps:
  - id: step1
    agent_ref: local://x
    depends_on: [step1]
"#;
        let err = WorkflowSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("cannot depend on itself"));
    }

    #[test]
    fn error_dependency_cycle() {
        let yaml = r#"
name: cyclic
version: 0.1.0
steps:
  - id: a
    agent_ref: local://a
    depends_on: [b]
  - id: b
    agent_ref: local://b
    depends_on: [c]
  - id: c
    agent_ref: local://c
    depends_on: [a]
"#;
        let err = WorkflowSpec::parse(yaml, "test").unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn topological_sort_linear_chain() {
        let spec = WorkflowSpec::parse(simple_workflow_yaml(), "test").unwrap();
        let order = topological_sort(&spec.steps).unwrap();
        let pos = |id: &str| spec.steps[order.iter().position(|&i| spec.steps[i].id == id).unwrap()].id.as_str();
        assert_eq!(pos("router"), "router");
        let router_pos = order.iter().position(|&i| spec.steps[i].id == "router").unwrap();
        let worker_pos = order.iter().position(|&i| spec.steps[i].id == "worker").unwrap();
        assert!(router_pos < worker_pos);
    }

    #[test]
    fn topological_sort_no_deps_returns_all() {
        let steps = vec![
            WorkflowStep {
                id: "a".into(),
                agent_ref: "local://a".into(),
                input_template: "{{ input }}".into(),
                depends_on: vec![],
                condition: None,
                timeout_ms: None,
            },
            WorkflowStep {
                id: "b".into(),
                agent_ref: "local://b".into(),
                input_template: "{{ input }}".into(),
                depends_on: vec![],
                condition: None,
                timeout_ms: None,
            },
        ];
        let order = topological_sort(&steps).unwrap();
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn topological_sort_diamond() {
        let yaml = r#"
name: diamond
version: 0.1.0
steps:
  - id: root
    agent_ref: local://root
  - id: left
    agent_ref: local://left
    depends_on: [root]
  - id: right
    agent_ref: local://right
    depends_on: [root]
  - id: sink
    agent_ref: local://sink
    depends_on: [left, right]
"#;
        let spec = WorkflowSpec::parse(yaml, "test").unwrap();
        let order = topological_sort(&spec.steps).unwrap();
        let pos = |id: &str| order.iter().position(|&i| spec.steps[i].id == id).unwrap();
        assert!(pos("root") < pos("left"));
        assert!(pos("root") < pos("right"));
        assert!(pos("left") < pos("sink"));
        assert!(pos("right") < pos("sink"));
    }

    #[test]
    fn topological_sort_cycle_returns_none() {
        let steps = vec![
            WorkflowStep {
                id: "x".into(),
                agent_ref: "local://x".into(),
                input_template: "{{ input }}".into(),
                depends_on: vec!["y".into()],
                condition: None,
                timeout_ms: None,
            },
            WorkflowStep {
                id: "y".into(),
                agent_ref: "local://y".into(),
                input_template: "{{ input }}".into(),
                depends_on: vec!["x".into()],
                condition: None,
                timeout_ms: None,
            },
        ];
        assert!(topological_sort(&steps).is_none());
    }

    #[test]
    fn load_missing_file_returns_io_error() {
        let err = WorkflowSpec::load(Path::new("/nonexistent/workflow.yaml")).unwrap_err();
        assert!(err.to_string().contains("IO error"));
    }

    #[test]
    fn invalid_yaml_returns_parse_error() {
        let err = WorkflowSpec::parse("not: valid: yaml: [", "test.yaml").unwrap_err();
        assert!(err.to_string().contains("Parse error"));
    }
}
