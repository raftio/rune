use std::collections::HashMap;
use std::time::Duration;

use uuid::Uuid;

use rune_spec::workflow::{topological_sort, WorkflowSpec};

use crate::error::RuntimeError;

/// Executes a workflow DAG by dispatching steps to agents via A2A.
pub struct WorkflowExecutor {
    spec: WorkflowSpec,
    call_depth: u32,
    gateway_base_url: String,
}

impl WorkflowExecutor {
    pub fn new(spec: WorkflowSpec) -> Self {
        Self {
            spec,
            call_depth: 0,
            gateway_base_url: "http://localhost:3000".into(),
        }
    }

    pub fn with_depth(spec: WorkflowSpec, depth: u32) -> Self {
        Self {
            spec,
            call_depth: depth,
            gateway_base_url: "http://localhost:3000".into(),
        }
    }

    pub fn with_gateway_base_url(mut self, url: String) -> Self {
        self.gateway_base_url = url;
        self
    }

    /// Run the workflow DAG to completion and return the final output.
    pub async fn run(
        &self,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let order = topological_sort(&self.spec.steps).ok_or_else(|| {
            RuntimeError::Engine("workflow has a dependency cycle".into())
        })?;

        let mut step_outputs: HashMap<String, serde_json::Value> = HashMap::new();
        let steps = &self.spec.steps;

        // Group steps by "wave" — steps in the same wave have all deps satisfied
        // and can run in parallel.
        let waves = build_waves(&order, steps);

        let overall_timeout = Duration::from_millis(self.spec.timeout_ms);
        let deadline = tokio::time::Instant::now() + overall_timeout;

        for wave in &waves {
            if tokio::time::Instant::now() > deadline {
                return Err(RuntimeError::Engine(format!(
                    "workflow '{}' timed out after {}ms",
                    self.spec.name, self.spec.timeout_ms
                )));
            }

            let mut handles = Vec::new();

            for &step_idx in wave {
                let step = &steps[step_idx];

                // Check condition.
                if let Some(ref cond) = step.condition {
                    if !evaluate_condition(cond, &step_outputs) {
                        tracing::info!(
                            workflow = %self.spec.name,
                            step = %step.id,
                            "skipping step — condition not met"
                        );
                        step_outputs.insert(
                            step.id.clone(),
                            serde_json::json!({ "skipped": true }),
                        );
                        continue;
                    }
                }

                let step_input = render_template(&step.input_template, &input, &step_outputs);
                let endpoint = resolve_agent_endpoint(&step.agent_ref, &self.gateway_base_url);
                let step_timeout = step
                    .timeout_ms
                    .unwrap_or(self.spec.timeout_ms);
                let step_id = step.id.clone();
                let depth = self.call_depth;

                handles.push(tokio::spawn(async move {
                    let client =
                        rune_a2a::A2aClient::new(Duration::from_millis(step_timeout));

                    let message = rune_a2a::Message {
                        message_id: Uuid::new_v4().to_string(),
                        role: rune_a2a::Role::User,
                        parts: vec![rune_a2a::Part::text(step_input)],
                        metadata: Some(serde_json::json!({
                            "rune_call_depth": depth + 1,
                            "rune_workflow_step": step_id,
                        })),
                        ..Default::default()
                    };

                    let result = client
                        .send_message_blocking(&endpoint, message)
                        .await
                        .map_err(|e| {
                            RuntimeError::Engine(format!(
                                "workflow step '{}' failed: {}",
                                step_id, e
                            ))
                        })?;

                    let output = match result {
                        rune_a2a::SendMessageResult::Task(task) => {
                            task.text_output()
                                .map(|t| serde_json::json!({ "text": t }))
                                .or_else(|| task.data_output())
                                .unwrap_or(serde_json::json!({ "completed": true }))
                        }
                        rune_a2a::SendMessageResult::Message(msg) => {
                            extract_message_output(&msg)
                        }
                    };

                    Ok::<(String, serde_json::Value), RuntimeError>((step_id, output))
                }));
            }

            // Await all parallel steps in this wave.
            for handle in handles {
                let (step_id, output) = handle
                    .await
                    .map_err(|e| RuntimeError::Engine(format!("step join error: {e}")))?
                    ?;
                step_outputs.insert(step_id, output);
            }
        }

        // Compose final output.
        let final_output = if let Some(ref output_step) = self.spec.output_step {
            step_outputs
                .get(output_step)
                .cloned()
                .unwrap_or(serde_json::json!({ "error": format!("output step '{}' not found", output_step) }))
        } else if let Some(last_idx) = order.last() {
            let last_id = &steps[*last_idx].id;
            step_outputs
                .get(last_id)
                .cloned()
                .unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        Ok(final_output)
    }
}

/// Group topologically-sorted indices into "waves" where all deps are
/// satisfied within or before the wave.
fn build_waves(order: &[usize], steps: &[rune_spec::workflow::WorkflowStep]) -> Vec<Vec<usize>> {
    use std::collections::HashSet;

    let id_to_idx: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    let mut completed: HashSet<usize> = HashSet::new();
    let mut waves: Vec<Vec<usize>> = Vec::new();
    let mut remaining: Vec<usize> = order.to_vec();

    while !remaining.is_empty() {
        let mut wave = Vec::new();
        let mut next_remaining = Vec::new();

        for &idx in &remaining {
            let step = &steps[idx];
            let deps_met = step
                .depends_on
                .iter()
                .all(|dep| {
                    id_to_idx
                        .get(dep.as_str())
                        .map(|&d| completed.contains(&d))
                        .unwrap_or(true)
                });

            if deps_met {
                wave.push(idx);
            } else {
                next_remaining.push(idx);
            }
        }

        if wave.is_empty() {
            // Should not happen if topological sort passed, but guard anyway.
            break;
        }

        for &idx in &wave {
            completed.insert(idx);
        }
        waves.push(wave);
        remaining = next_remaining;
    }

    waves
}

/// Simple template substitution.
fn render_template(
    template: &str,
    input: &serde_json::Value,
    step_outputs: &HashMap<String, serde_json::Value>,
) -> String {
    let mut result = template.to_string();

    // Replace {{ input }}
    let input_str = match input {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    result = result.replace("{{ input }}", &input_str);

    // Replace {{ steps.<id>.output }}
    for (step_id, output) in step_outputs {
        // Reject step_ids with characters that could interfere with the
        // template syntax or inject unexpected patterns.
        if !step_id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            tracing::warn!(step_id = %step_id, "skipping step_id with unsafe characters in template");
            continue;
        }
        let pattern = format!("{{{{ steps.{step_id}.output }}}}");
        let output_str = match output {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(obj) => {
                obj.get("text")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| output.to_string())
            }
            other => other.to_string(),
        };
        result = result.replace(&pattern, &output_str);
    }

    result
}

/// Simple condition evaluator: `{{ steps.<id>.output }} == "value"`
fn evaluate_condition(
    condition: &str,
    step_outputs: &HashMap<String, serde_json::Value>,
) -> bool {
    if let Some((lhs, rhs)) = condition.split_once("==") {
        let lhs = lhs.trim();
        let rhs = rhs.trim().trim_matches('"');

        let lhs_val = resolve_template_value(lhs, step_outputs);
        lhs_val.as_deref() == Some(rhs)
    } else {
        // No operator — check if the referenced value is truthy.
        resolve_template_value(condition.trim(), step_outputs)
            .map(|v| !v.is_empty() && v != "false" && v != "null")
            .unwrap_or(false)
    }
}

fn resolve_template_value(
    expr: &str,
    step_outputs: &HashMap<String, serde_json::Value>,
) -> Option<String> {
    let expr = expr
        .trim_start_matches("{{")
        .trim_end_matches("}}")
        .trim();

    // steps.<id>.output
    if let Some(rest) = expr.strip_prefix("steps.") {
        if let Some((step_id, _field)) = rest.split_once('.') {
            return step_outputs.get(step_id).map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(obj) => {
                    obj.get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string()
                }
                other => other.to_string(),
            });
        }
    }
    None
}

fn resolve_agent_endpoint(agent_ref: &str, gateway_base_url: &str) -> String {
    if let Some(name) = agent_ref.strip_prefix("local://") {
        format!("{}/a2a/{}", gateway_base_url.trim_end_matches('/'), name)
    } else {
        agent_ref.to_string()
    }
}

fn extract_message_output(msg: &rune_a2a::Message) -> serde_json::Value {
    for part in &msg.parts {
        match part {
            rune_a2a::Part::Data { data, .. } => return data.clone(),
            rune_a2a::Part::Text { text, .. } => {
                return serde_json::json!({ "text": text });
            }
            _ => {}
        }
    }
    serde_json::json!({ "message": "empty response" })
}
