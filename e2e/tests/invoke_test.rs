mod common;

use common::TestServer;
use serde_json::json;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Invoke API (POST /v1/agents/:name/invoke)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_invoke_creates_session() {
    let srv = TestServer::start().await;
    srv.seed_agent("invoke-agent").await;

    let (status, body) = srv
        .post_json(
            "/v1/agents/invoke-agent/invoke",
            &json!({
                "input": { "text": "hello" }
            }),
        )
        .await;

    assert_eq!(status, 200, "body: {body}");
    assert!(body["session_id"].as_str().is_some(), "should have session_id");
    assert!(body["request_id"].as_str().is_some(), "should have request_id");
    assert!(body["output"].is_object(), "should have output");
}

#[tokio::test]
async fn test_invoke_with_session_reuses() {
    let srv = TestServer::start().await;
    srv.seed_agent("reuse-agent").await;

    // Create session first.
    let (create_status, create_body) = srv
        .post_json(
            "/v1/agents/reuse-agent/sessions",
            &json!({ "agent_name": "reuse-agent" }),
        )
        .await;
    assert_eq!(create_status, 201, "create session: {create_body}");
    let session_id = create_body["session_id"].as_str().expect("session_id").to_string();

    // Invoke with that session.
    let (invoke_status, invoke_body) = srv
        .post_json(
            "/v1/agents/reuse-agent/invoke",
            &json!({
                "session_id": session_id,
                "input": { "text": "hi" }
            }),
        )
        .await;

    assert_eq!(invoke_status, 200, "invoke: {invoke_body}");
    assert_eq!(invoke_body["session_id"].as_str().unwrap(), session_id);
}

#[tokio::test]
async fn test_invoke_no_replica_returns_503() {
    let srv = TestServer::start().await;
    let cp = srv.cp_url();

    // Register version + create deployment, but do NOT insert replica.
    let resp: serde_json::Value = srv
        .client
        .post(format!("{cp}/v1/agent-versions"))
        .json(&json!({
            "agent_name": "no-replica-agent",
            "version": "0.1.0",
            "image_ref": "local/no-replica-agent",
            "image_digest": format!("sha256:{}", Uuid::new_v4()),
            "spec_sha256": format!("{}", Uuid::new_v4()),
            "runtime_class": "wasm",
        }))
        .send()
        .await
        .expect("register")
        .json()
        .await
        .expect("parse");

    let version_id = resp["id"].as_str().unwrap();
    let deploy_resp: serde_json::Value = srv
        .client
        .post(format!("{cp}/v1/deployments"))
        .json(&json!({
            "agent_version_id": version_id,
            "namespace": "test",
            "rollout_alias": "stable",
        }))
        .send()
        .await
        .expect("deploy")
        .json()
        .await
        .expect("parse");

    let deployment_id = deploy_resp["id"].as_str().unwrap();
    sqlx::query("UPDATE agent_deployments SET status = 'active' WHERE id = ?")
        .bind(deployment_id)
        .execute(&srv.db)
        .await
        .expect("activate");
    // No replica inserted.

    let (status, body) = srv
        .post_json(
            "/v1/agents/no-replica-agent/invoke",
            &json!({ "input": { "text": "x" } }),
        )
        .await;

    assert_eq!(status, 503, "expected 503 when no replica: {body}");
    assert!(body["error"].as_str().is_some());
}

#[tokio::test]
async fn test_invoke_agent_not_found_404() {
    let srv = TestServer::start().await;
    // Do not seed any agent.

    let (status, body) = srv
        .post_json(
            "/v1/agents/nonexistent/invoke",
            &json!({ "input": { "text": "x" } }),
        )
        .await;

    assert_eq!(status, 404, "expected 404 for unknown agent: {body}");
}

// ---------------------------------------------------------------------------
// Sessions API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_create_session() {
    let srv = TestServer::start().await;
    srv.seed_agent("session-agent").await;

    let (status, body) = srv
        .post_json(
            "/v1/agents/session-agent/sessions",
            &json!({ "agent_name": "session-agent" }),
        )
        .await;

    assert_eq!(status, 201, "body: {body}");
    assert!(body["session_id"].as_str().is_some());
    assert!(body["deployment_id"].as_str().is_some());
    assert_eq!(body["status"].as_str().unwrap(), "active");
}

#[tokio::test]
async fn test_get_session() {
    let srv = TestServer::start().await;
    srv.seed_agent("get-session-agent").await;

    let (_, create_body) = srv
        .post_json(
            "/v1/agents/get-session-agent/sessions",
            &json!({ "agent_name": "get-session-agent" }),
        )
        .await;
    let session_id = create_body["session_id"].as_str().unwrap();

    let (status, body) = srv.get_json(&format!("/v1/sessions/{session_id}")).await;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["id"].as_str().unwrap(), session_id);
    assert_eq!(body["status"].as_str().unwrap(), "active");
}

#[tokio::test]
async fn test_get_session_not_found() {
    let srv = TestServer::start().await;
    let fake_id = Uuid::new_v4();

    let (status, _body) = srv
        .get_json(&format!("/v1/sessions/{fake_id}"))
        .await;

    assert_eq!(status, 404);
}
