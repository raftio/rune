mod common;

use common::TestServer;
use serde_json::json;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Agent Card
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_global_agent_card() {
    let srv = TestServer::start().await;
    srv.seed_agent("weather").await;

    let (status, body) = srv.get_json("/.well-known/agent.json").await;
    assert_eq!(status, 200, "body: {body}");

    assert_eq!(body["name"], "Rune Agent Runtime");
    assert!(body["capabilities"]["streaming"].as_bool().unwrap());

    let skills = body["skills"].as_array().expect("skills should be array");
    let names: Vec<&str> = skills.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(names.contains(&"weather"), "seeded agent should appear in skills: {names:?}");
}

#[tokio::test]
async fn test_per_agent_card() {
    let srv = TestServer::start().await;
    srv.seed_agent("translator").await;

    let (status, body) = srv.get_json("/a2a/translator/agent-card").await;
    assert_eq!(status, 200, "body: {body}");

    assert_eq!(body["name"], "translator");
    assert!(body["capabilities"]["streaming"].as_bool().unwrap());

    let ifaces = body["supportedInterfaces"].as_array().expect("interfaces");
    assert!(!ifaces.is_empty());
    let url = ifaces[0]["url"].as_str().unwrap();
    assert!(url.contains("/a2a/translator"), "url should contain agent name: {url}");
}

#[tokio::test]
async fn test_agent_card_not_found() {
    let srv = TestServer::start().await;

    let (status, _body) = srv.get_json("/a2a/nonexistent/agent-card").await;
    assert_eq!(status, 404);
}

// ---------------------------------------------------------------------------
// message/send
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_message_send() {
    let srv = TestServer::start().await;
    srv.seed_agent("echo").await;

    let resp = srv
        .a2a_rpc(
            "echo",
            "message/send",
            json!({
                "message": {
                    "messageId": Uuid::new_v4().to_string(),
                    "role": "user",
                    "parts": [{"kind": "text", "text": "hello"}]
                }
            }),
        )
        .await;

    assert!(resp.get("error").is_none(), "unexpected error: {resp}");

    let result = &resp["result"];
    assert_eq!(result["kind"], "task");
    assert_eq!(result["status"]["state"], "completed");

    let artifacts = result["artifacts"].as_array().expect("artifacts");
    assert!(!artifacts.is_empty(), "should have artifacts");

    assert!(result["contextId"].as_str().is_some(), "contextId missing");

    let text = artifacts[0]["parts"][0]["text"].as_str().unwrap_or("");
    assert!(!text.is_empty(), "artifact text should not be empty");
}

#[tokio::test]
async fn test_message_send_with_context() {
    let srv = TestServer::start().await;
    srv.seed_agent("ctx-agent").await;

    // First call: no context_id.
    let resp1 = srv
        .a2a_rpc(
            "ctx-agent",
            "message/send",
            json!({
                "message": {
                    "messageId": Uuid::new_v4().to_string(),
                    "role": "user",
                    "parts": [{"kind": "text", "text": "first"}]
                }
            }),
        )
        .await;

    assert!(resp1.get("error").is_none(), "first call error: {resp1}");
    let ctx_id = resp1["result"]["contextId"]
        .as_str()
        .expect("contextId on first call")
        .to_string();

    // Second call: same context_id.
    let resp2 = srv
        .a2a_rpc(
            "ctx-agent",
            "message/send",
            json!({
                "message": {
                    "messageId": Uuid::new_v4().to_string(),
                    "contextId": ctx_id,
                    "role": "user",
                    "parts": [{"kind": "text", "text": "second"}]
                }
            }),
        )
        .await;

    assert!(resp2.get("error").is_none(), "second call error: {resp2}");

    let ctx_id2 = resp2["result"]["contextId"]
        .as_str()
        .expect("contextId on second call");
    assert_eq!(ctx_id, ctx_id2, "context should be reused");
}

// ---------------------------------------------------------------------------
// tasks/get
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tasks_get() {
    let srv = TestServer::start().await;
    srv.seed_agent("getter").await;

    // Create a task via message/send.
    let resp = srv
        .a2a_rpc(
            "getter",
            "message/send",
            json!({
                "message": {
                    "messageId": Uuid::new_v4().to_string(),
                    "role": "user",
                    "parts": [{"kind": "text", "text": "test get"}]
                }
            }),
        )
        .await;

    let task_id = resp["result"]["id"]
        .as_str()
        .expect("task id")
        .to_string();

    // Now fetch it.
    let get_resp = srv
        .a2a_rpc("getter", "tasks/get", json!({ "id": task_id }))
        .await;

    assert!(get_resp.get("error").is_none(), "get error: {get_resp}");
    let result = &get_resp["result"];
    assert_eq!(result["id"], task_id);
    assert_eq!(result["status"]["state"], "completed");
}

// ---------------------------------------------------------------------------
// tasks/cancel
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tasks_cancel() {
    let srv = TestServer::start().await;
    srv.seed_agent("canceler").await;

    // Insert a task in 'submitted' state directly in DB to have something cancelable.
    let task_id = Uuid::new_v4().to_string();
    let ctx_id = Uuid::new_v4().to_string();
    let req_id = Uuid::new_v4().to_string();
    let session_id = Uuid::new_v4().to_string();

    // We need a session and a request for the foreign keys.
    // First get the deployment_id.
    let dep_id: String = sqlx::query_scalar(
        "SELECT d.id FROM agent_deployments d
         JOIN agent_versions v ON v.id = d.agent_version_id
         WHERE v.agent_name = 'canceler' AND d.status = 'active' LIMIT 1",
    )
    .fetch_one(&srv.db)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO agent_sessions (id, deployment_id) VALUES (?, ?)",
    )
    .bind(&session_id)
    .bind(&dep_id)
    .execute(&srv.db)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO agent_requests (id, session_id, deployment_id, request_payload, status)
         VALUES (?, ?, ?, '{}', 'running')",
    )
    .bind(&req_id)
    .bind(&session_id)
    .bind(&dep_id)
    .execute(&srv.db)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO a2a_tasks (task_id, context_id, request_id, session_id, agent_name, state)
         VALUES (?, ?, ?, ?, 'canceler', 'submitted')",
    )
    .bind(&task_id)
    .bind(&ctx_id)
    .bind(&req_id)
    .bind(&session_id)
    .execute(&srv.db)
    .await
    .unwrap();

    let resp = srv
        .a2a_rpc("canceler", "tasks/cancel", json!({ "id": task_id }))
        .await;

    assert!(resp.get("error").is_none(), "cancel error: {resp}");
    assert_eq!(resp["result"]["status"]["state"], "canceled");
}

#[tokio::test]
async fn test_tasks_cancel_already_completed() {
    let srv = TestServer::start().await;
    srv.seed_agent("done-agent").await;

    // Create a completed task via message/send, then try to cancel.
    let send_resp = srv
        .a2a_rpc(
            "done-agent",
            "message/send",
            json!({
                "message": {
                    "messageId": Uuid::new_v4().to_string(),
                    "role": "user",
                    "parts": [{"kind": "text", "text": "done"}]
                }
            }),
        )
        .await;

    let task_id = send_resp["result"]["id"]
        .as_str()
        .expect("task id")
        .to_string();

    let cancel_resp = srv
        .a2a_rpc("done-agent", "tasks/cancel", json!({ "id": task_id }))
        .await;

    // Already completed — should return an error.
    assert!(
        cancel_resp.get("error").is_some(),
        "expected error for completed task cancel: {cancel_resp}"
    );
    assert_eq!(cancel_resp["error"]["code"], -32002);
}

// ---------------------------------------------------------------------------
// Depth protection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_depth_protection() {
    let srv = TestServer::start().await;
    srv.seed_agent("deep").await;

    let resp = srv
        .a2a_rpc(
            "deep",
            "message/send",
            json!({
                "message": {
                    "messageId": Uuid::new_v4().to_string(),
                    "role": "user",
                    "parts": [{"kind": "text", "text": "recursive"}],
                    "metadata": { "rune_call_depth": 11 }
                }
            }),
        )
        .await;

    assert!(resp.get("error").is_some(), "expected depth error: {resp}");
    assert_eq!(resp["error"]["code"], -32602, "error code should be INVALID_PARAMS");
    let msg = resp["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("depth"), "error message should mention depth: {msg}");
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unknown_method() {
    let srv = TestServer::start().await;
    srv.seed_agent("any").await;

    let resp = srv
        .a2a_rpc("any", "foo/bar", json!({}))
        .await;

    assert!(resp.get("error").is_some(), "expected error: {resp}");
    assert_eq!(resp["error"]["code"], -32601, "METHOD_NOT_FOUND");
}

#[tokio::test]
async fn test_invalid_params() {
    let srv = TestServer::start().await;
    srv.seed_agent("bad-params").await;

    // message/send without the required `message` field.
    let resp = srv
        .a2a_rpc(
            "bad-params",
            "message/send",
            json!({ "not_a_message": true }),
        )
        .await;

    // The gateway returns a 500 with the error in the body because
    // serde parse failure triggers GatewayError::Internal, which axum
    // renders as a JSON error response. We just verify it's not a success.
    let has_rpc_error = resp.get("error").is_some();
    let is_http_error = resp.get("error").map(|e| e.is_string()).unwrap_or(false);
    assert!(
        has_rpc_error || is_http_error,
        "expected error for invalid params: {resp}"
    );
}
