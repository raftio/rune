//! Workflow DAG types for multi-step agent orchestration.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
    #[serde(default = "default_workflow_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub output_step: Option<String>,
}

fn default_workflow_timeout_ms() -> u64 {
    300_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub agent_ref: String,
    pub input_template: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Topological order of step indices, or `None` if the dependency graph has a cycle.
pub fn topological_sort(steps: &[WorkflowStep]) -> Option<Vec<usize>> {
    let n = steps.len();
    if n == 0 {
        return Some(vec![]);
    }

    let id_to_idx: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    let mut indegree = vec![0u32; n];
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];

    for (i, step) in steps.iter().enumerate() {
        for dep in &step.depends_on {
            let j = *id_to_idx.get(dep.as_str())?;
            adj[j].push(i);
            indegree[i] += 1;
        }
    }

    let mut q: VecDeque<usize> = indegree
        .iter()
        .enumerate()
        .filter(|(_, &d)| d == 0)
        .map(|(i, _)| i)
        .collect();

    let mut out = Vec::with_capacity(n);
    while let Some(u) = q.pop_front() {
        out.push(u);
        for &v in &adj[u] {
            indegree[v] -= 1;
            if indegree[v] == 0 {
                q.push_back(v);
            }
        }
    }

    if out.len() != n {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topo_empty() {
        assert_eq!(topological_sort(&[]), Some(vec![]));
    }

    #[test]
    fn topo_linear() {
        let steps = vec![
            WorkflowStep {
                id: "a".into(),
                agent_ref: "local://x".into(),
                input_template: "{}".into(),
                depends_on: vec![],
                condition: None,
                timeout_ms: None,
            },
            WorkflowStep {
                id: "b".into(),
                agent_ref: "local://x".into(),
                input_template: "{}".into(),
                depends_on: vec!["a".into()],
                condition: None,
                timeout_ms: None,
            },
        ];
        let order = topological_sort(&steps).unwrap();
        assert_eq!(order, vec![0, 1]);
    }

    #[test]
    fn topo_cycle_returns_none() {
        let steps = vec![
            WorkflowStep {
                id: "a".into(),
                agent_ref: "local://x".into(),
                input_template: "{}".into(),
                depends_on: vec!["b".into()],
                condition: None,
                timeout_ms: None,
            },
            WorkflowStep {
                id: "b".into(),
                agent_ref: "local://x".into(),
                input_template: "{}".into(),
                depends_on: vec!["a".into()],
                condition: None,
                timeout_ms: None,
            },
        ];
        assert!(topological_sort(&steps).is_none());
    }
}
