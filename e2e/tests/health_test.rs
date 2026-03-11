mod common;

use common::TestServer;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Health endpoint (GET /v1/replicas/:id/health)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_health_no_backend() {
    let srv = TestServer::start().await;
    srv.seed_agent("health-agent").await;

    // Get a replica id from the seeded deployment.
    let replica_id: String = sqlx::query_scalar(
        "SELECT id FROM agent_replicas
         WHERE deployment_id IN (
           SELECT d.id FROM agent_deployments d
           JOIN agent_versions v ON v.id = d.agent_version_id
           WHERE v.agent_name = 'health-agent'
         )
         LIMIT 1",
    )
    .fetch_one(&srv.db)
    .await
    .expect("replica");

    let (status, body) = srv
        .get_json(&format!("/v1/replicas/{replica_id}/health"))
        .await;

    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["healthy"].as_bool(), Some(false));
    assert!(
        body["message"].as_str().is_some(),
        "should have message when no backend"
    );
}

#[tokio::test]
async fn test_health_replica_not_found() {
    let srv = TestServer::start().await;
    let fake_id = Uuid::new_v4();

    let (status, body) = srv
        .get_json(&format!("/v1/replicas/{fake_id}/health"))
        .await;

    // With no backend, health route returns 200 with healthy: false.
    // It does not 404 because it doesn't check replica existence when backend is None.
    assert_eq!(status, 200);
    assert_eq!(body["healthy"].as_bool(), Some(false));
}
