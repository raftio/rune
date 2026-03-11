mod common;

use common::TestServer;
use serde_json::json;
use uuid::Uuid;

/// Register an agent version and return its ID.
async fn register_version(srv: &TestServer, agent_name: &str) -> String {
    let cp = srv.cp_url();
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("{cp}/v1/agent-versions"))
        .json(&json!({
            "agent_name": agent_name,
            "version": "0.1.0",
            "image_ref": format!("local/{agent_name}"),
            "image_digest": format!("sha256:{}", Uuid::new_v4()),
            "spec_sha256": format!("{}", Uuid::new_v4()),
            "runtime_class": "wasm",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    resp["id"].as_str().unwrap().to_string()
}

/// Create a deployment tagged with a project_ref.
async fn create_deployment(
    srv: &TestServer,
    version_id: &str,
    project: &str,
) -> serde_json::Value {
    let cp = srv.cp_url();
    reqwest::Client::new()
        .post(format!("{cp}/v1/deployments"))
        .json(&json!({
            "agent_version_id": version_id,
            "namespace": "test",
            "rollout_alias": "stable",
            "project_ref": project,
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// project_ref on deployment creation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_deployment_project_ref_persisted() {
    let srv = TestServer::start().await;
    let vid = register_version(&srv, "proj-agent").await;
    let deploy = create_deployment(&srv, &vid, "my-project").await;

    assert_eq!(deploy["project_ref"].as_str().unwrap(), "my-project");
    assert_eq!(deploy["status"].as_str().unwrap(), "pending");
}

// ---------------------------------------------------------------------------
// filter deployments by project
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_deployments_filter_by_project() {
    let srv = TestServer::start().await;

    let v1 = register_version(&srv, "agent-a").await;
    let v2 = register_version(&srv, "agent-b").await;
    let v3 = register_version(&srv, "agent-c").await;

    create_deployment(&srv, &v1, "project-x").await;
    create_deployment(&srv, &v2, "project-x").await;
    create_deployment(&srv, &v3, "project-y").await;

    let cp = srv.cp_url();
    let http = reqwest::Client::new();

    // Filter by project-x: should return 2
    let body: serde_json::Value = http
        .get(format!("{cp}/v1/deployments?project=project-x"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deps = body["deployments"].as_array().unwrap();
    assert_eq!(deps.len(), 2, "project-x should have 2 deployments");

    // Filter by project-y: should return 1
    let body: serde_json::Value = http
        .get(format!("{cp}/v1/deployments?project=project-y"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deps = body["deployments"].as_array().unwrap();
    assert_eq!(deps.len(), 1, "project-y should have 1 deployment");

    // No filter: should return all 3
    let body: serde_json::Value = http
        .get(format!("{cp}/v1/deployments"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deps = body["deployments"].as_array().unwrap();
    assert_eq!(deps.len(), 3, "unfiltered should have 3 deployments");
}

// ---------------------------------------------------------------------------
// delete deployment
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_delete_deployment() {
    let srv = TestServer::start().await;
    let vid = register_version(&srv, "del-agent").await;
    let deploy = create_deployment(&srv, &vid, "del-project").await;
    let deploy_id = deploy["id"].as_str().unwrap();

    let cp = srv.cp_url();
    let http = reqwest::Client::new();

    let resp = http
        .delete(format!("{cp}/v1/deployments/{deploy_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 204);

    // Verify it's gone
    let body: serde_json::Value = http
        .get(format!("{cp}/v1/deployments?project=del-project"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deps = body["deployments"].as_array().unwrap();
    assert!(deps.is_empty(), "deployment should be deleted");
}

#[tokio::test]
async fn test_delete_nonexistent_deployment() {
    let srv = TestServer::start().await;
    let cp = srv.cp_url();
    let fake_id = Uuid::new_v4();

    let resp = reqwest::Client::new()
        .delete(format!("{cp}/v1/deployments/{fake_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

// ---------------------------------------------------------------------------
// compose lifecycle: create multiple, list by project, delete all
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_compose_lifecycle() {
    let srv = TestServer::start().await;
    let project = "compose-lifecycle";

    let v1 = register_version(&srv, "svc-alpha").await;
    let v2 = register_version(&srv, "svc-beta").await;
    let v3 = register_version(&srv, "svc-gamma").await;

    // "compose up" — deploy all three with the same project_ref
    let d1 = create_deployment(&srv, &v1, project).await;
    let d2 = create_deployment(&srv, &v2, project).await;
    let d3 = create_deployment(&srv, &v3, project).await;

    let cp = srv.cp_url();
    let http = reqwest::Client::new();

    // "compose ps" — list by project
    let body: serde_json::Value = http
        .get(format!("{cp}/v1/deployments?project={project}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deps = body["deployments"].as_array().unwrap();
    assert_eq!(deps.len(), 3);

    // "compose down" — delete all project deployments
    for deploy in [&d1, &d2, &d3] {
        let id = deploy["id"].as_str().unwrap();
        let resp = http
            .delete(format!("{cp}/v1/deployments/{id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 204, "failed to delete {id}");
    }

    // Verify all gone
    let body: serde_json::Value = http
        .get(format!("{cp}/v1/deployments?project={project}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deps = body["deployments"].as_array().unwrap();
    assert!(deps.is_empty(), "all deployments should be removed after compose down");
}

// ---------------------------------------------------------------------------
// scale deployment
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_scale_deployment() {
    let srv = TestServer::start().await;
    let vid = register_version(&srv, "scale-agent").await;
    let deploy = create_deployment(&srv, &vid, "scale-project").await;
    let deploy_id = deploy["id"].as_str().unwrap();

    let cp = srv.cp_url();
    let http = reqwest::Client::new();

    let body: serde_json::Value = http
        .post(format!("{cp}/v1/deployments/{deploy_id}/scale"))
        .json(&json!({ "desired_replicas": 5 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["id"].as_str().unwrap(), deploy_id);
    assert_eq!(body["desired_replicas"].as_i64().unwrap(), 5);

    // Verify persisted
    let list: serde_json::Value = http
        .get(format!("{cp}/v1/deployments?project=scale-project"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deps = list["deployments"].as_array().unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0]["desired_replicas"].as_i64().unwrap(), 5);
}

// ---------------------------------------------------------------------------
// register version validation (missing required fields)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_version_validation() {
    let srv = TestServer::start().await;
    let cp = srv.cp_url();
    let http = reqwest::Client::new();

    // Omit required fields (spec_sha256, image_digest) - serde deserialization fails
    let resp = http
        .post(format!("{cp}/v1/agent-versions"))
        .json(&json!({
            "agent_name": "minimal",
            "version": "0.1.0",
            "image_ref": "x",
            "runtime_class": "wasm",
        }))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_client_error(),
        "missing required fields should return 4xx"
    );
}

// ---------------------------------------------------------------------------
// deployment without project_ref (backward compatibility)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_deployment_without_project_ref() {
    let srv = TestServer::start().await;
    let vid = register_version(&srv, "no-proj-agent").await;

    let cp = srv.cp_url();
    let deploy: serde_json::Value = reqwest::Client::new()
        .post(format!("{cp}/v1/deployments"))
        .json(&json!({
            "agent_version_id": vid,
            "namespace": "test",
            "rollout_alias": "stable",
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(
        deploy["project_ref"].is_null(),
        "project_ref should be null when not provided",
    );
}
