use std::time::Duration;

use crate::error::A2aError;
use crate::jsonrpc::*;
use crate::types::*;

/// A2A protocol HTTP client for agent-to-agent communication.
pub struct A2aClient {
    http: reqwest::Client,
    timeout: Duration,
}

impl A2aClient {
    pub fn new(timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build reqwest client");
        Self { http, timeout }
    }

    pub fn with_default_timeout() -> Self {
        Self::new(Duration::from_secs(60))
    }

    /// Fetch an agent's AgentCard from its well-known endpoint or explicit URL.
    pub async fn discover(&self, url: &str) -> Result<AgentCard, A2aError> {
        let card_url = if url.ends_with("/agent.json") || url.ends_with("/agent-card") {
            url.to_string()
        } else {
            format!(
                "{}/.well-known/agent.json",
                url.trim_end_matches('/')
            )
        };

        let resp = self
            .http
            .get(&card_url)
            .header("Accept", "application/json")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(A2aError::Discovery(format!(
                "HTTP {} from {}",
                resp.status(),
                card_url
            )));
        }

        let card: AgentCard = resp.json().await?;
        Ok(card)
    }

    /// Send a message to a remote agent via JSON-RPC `message/send`.
    pub async fn send_message(
        &self,
        endpoint: &str,
        message: Message,
        config: Option<SendMessageConfiguration>,
    ) -> Result<SendMessageResult, A2aError> {
        let params = SendMessageParams {
            message,
            configuration: config,
            metadata: None,
        };

        let rpc_req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(uuid::Uuid::new_v4().to_string()),
            method: METHOD_MESSAGE_SEND.into(),
            params: serde_json::to_value(&params)?,
        };

        let resp = self
            .http
            .post(endpoint)
            .json(&rpc_req)
            .send()
            .await?;

        let rpc_resp: JsonRpcResponse = resp.json().await?;

        if let Some(err) = rpc_resp.error {
            return Err(A2aError::JsonRpc {
                code: err.code,
                message: err.message,
            });
        }

        let result = rpc_resp
            .result
            .ok_or_else(|| A2aError::Other("empty result in JSON-RPC response".into()))?;

        let send_result: SendMessageResult = serde_json::from_value(result)?;
        Ok(send_result)
    }

    /// Retrieve the current state of a task via JSON-RPC `tasks/get`.
    pub async fn get_task(
        &self,
        endpoint: &str,
        task_id: &str,
        history_length: Option<u32>,
    ) -> Result<Task, A2aError> {
        let params = GetTaskParams {
            id: task_id.to_string(),
            history_length,
        };

        let rpc_req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(uuid::Uuid::new_v4().to_string()),
            method: METHOD_TASKS_GET.into(),
            params: serde_json::to_value(&params)?,
        };

        let resp = self
            .http
            .post(endpoint)
            .json(&rpc_req)
            .send()
            .await?;

        let rpc_resp: JsonRpcResponse = resp.json().await?;

        if let Some(err) = rpc_resp.error {
            return Err(A2aError::JsonRpc {
                code: err.code,
                message: err.message,
            });
        }

        let result = rpc_resp
            .result
            .ok_or_else(|| A2aError::Other("empty result in tasks/get response".into()))?;

        let task: Task = serde_json::from_value(result)?;
        Ok(task)
    }

    /// Cancel a task via JSON-RPC `tasks/cancel`.
    pub async fn cancel_task(
        &self,
        endpoint: &str,
        task_id: &str,
    ) -> Result<Task, A2aError> {
        let params = CancelTaskParams {
            id: task_id.to_string(),
            metadata: None,
        };

        let rpc_req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(uuid::Uuid::new_v4().to_string()),
            method: METHOD_TASKS_CANCEL.into(),
            params: serde_json::to_value(&params)?,
        };

        let resp = self
            .http
            .post(endpoint)
            .json(&rpc_req)
            .send()
            .await?;

        let rpc_resp: JsonRpcResponse = resp.json().await?;

        if let Some(err) = rpc_resp.error {
            return Err(A2aError::JsonRpc {
                code: err.code,
                message: err.message,
            });
        }

        let result = rpc_resp
            .result
            .ok_or_else(|| A2aError::Other("empty result in tasks/cancel response".into()))?;

        let task: Task = serde_json::from_value(result)?;
        Ok(task)
    }

    /// Send a message and poll until the task reaches a terminal state.
    /// Useful for agent-as-tool invocation where we need a synchronous result.
    pub async fn send_message_blocking(
        &self,
        endpoint: &str,
        message: Message,
    ) -> Result<SendMessageResult, A2aError> {
        let config = SendMessageConfiguration {
            blocking: Some(true),
            ..Default::default()
        };

        let result = self.send_message(endpoint, message, Some(config)).await?;

        match &result {
            SendMessageResult::Message(_) => return Ok(result),
            SendMessageResult::Task(task) => {
                if task.status.state == TaskState::Completed
                    || task.status.state == TaskState::Failed
                    || task.status.state == TaskState::Canceled
                    || task.status.state == TaskState::Rejected
                {
                    return Ok(result);
                }

                // Poll until terminal if the server didn't honour blocking=true.
                let task_id = &task.id;
                let deadline =
                    tokio::time::Instant::now() + self.timeout;
                let mut interval = tokio::time::interval(Duration::from_millis(500));

                loop {
                    interval.tick().await;
                    if tokio::time::Instant::now() > deadline {
                        return Err(A2aError::Timeout(self.timeout.as_millis() as u64));
                    }

                    let polled = self.get_task(endpoint, task_id, None).await?;
                    match polled.status.state {
                        TaskState::Completed
                        | TaskState::Failed
                        | TaskState::Canceled
                        | TaskState::Rejected => {
                            return Ok(SendMessageResult::Task(polled));
                        }
                        _ => continue,
                    }
                }
            }
        }
    }
}
