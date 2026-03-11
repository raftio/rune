use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

/// Self-contained test server: in-memory SQLite + gateway + control-plane.
/// Methods are used across different test files; each integration test is compiled
/// as a separate binary, so the compiler reports dead_code for unused items.
#[allow(dead_code)]
pub struct TestServer {
    pub db: SqlitePool,
    pub client: reqwest::Client,
    gw_addr: SocketAddr,
    cp_addr: SocketAddr,
}

#[allow(dead_code)]
impl TestServer {
    /// Spin up a fresh server with an isolated in-memory database.
    pub async fn start() -> Self {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
            .foreign_keys(true)
            .shared_cache(true);

        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("failed to create in-memory SQLite pool");

        rune_runtime::run_migrations(&db)
            .await
            .expect("migrations failed");

        let gw_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind gateway");
        let gw_addr = gw_listener.local_addr().unwrap();

        let cp_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind control-plane");
        let cp_addr = cp_listener.local_addr().unwrap();

        let store = Arc::new(rune_runtime::RuneStore::from_pool(db.clone()));
        let env = Arc::new(rune_env::PlatformEnv::from_env().expect("platform env"));
        let gw_router = rune_gateway::router(store.clone(), None, None, env);
        let cp_router = rune_cp::router(store);

        tokio::spawn(async move {
            axum::serve(gw_listener, gw_router).await.ok();
        });
        tokio::spawn(async move {
            axum::serve(cp_listener, cp_router).await.ok();
        });

        // Brief yield so the listeners are ready.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        Self {
            db,
            client: reqwest::Client::new(),
            gw_addr,
            cp_addr,
        }
    }

    pub fn gw_url(&self) -> String {
        format!("http://{}", self.gw_addr)
    }

    pub fn cp_url(&self) -> String {
        format!("http://{}", self.cp_addr)
    }

    /// Register an agent version + create an active deployment.
    /// Returns the deployment ID.
    pub async fn seed_agent(&self, agent_name: &str) -> String {
        let cp = self.cp_url();

        // 1. Register agent version.
        let resp: serde_json::Value = self
            .client
            .post(format!("{cp}/v1/agent-versions"))
            .json(&serde_json::json!({
                "agent_name": agent_name,
                "version": "0.1.0",
                "image_ref": format!("local/{agent_name}"),
                "image_digest": format!("sha256:{}", Uuid::new_v4()),
                "spec_sha256": format!("{}", Uuid::new_v4()),
                "runtime_class": "wasm",
            }))
            .send()
            .await
            .expect("register version request failed")
            .json()
            .await
            .expect("register version response parse failed");

        let version_id = resp["id"].as_str().expect("version id missing").to_string();

        // 2. Create deployment.
        let resp: serde_json::Value = self
            .client
            .post(format!("{cp}/v1/deployments"))
            .json(&serde_json::json!({
                "agent_version_id": version_id,
                "namespace": "test",
                "rollout_alias": "stable",
            }))
            .send()
            .await
            .expect("create deployment request failed")
            .json()
            .await
            .expect("create deployment response parse failed");

        let deployment_id = resp["id"].as_str().expect("deployment id missing").to_string();

        // 3. Force deployment to active (no reconcile loop in tests).
        sqlx::query("UPDATE agent_deployments SET status = 'active' WHERE id = ?")
            .bind(&deployment_id)
            .execute(&self.db)
            .await
            .expect("failed to activate deployment");

        // 4. Insert a mock "ready" replica so ReplicaRouter::acquire() succeeds.
        let replica_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO agent_replicas
                (id, deployment_id, backend_type, backend_instance_id, state, concurrency_limit, current_load)
             VALUES (?, ?, 'wasm', ?, 'ready', 10, 0)",
        )
        .bind(&replica_id)
        .bind(&deployment_id)
        .bind(format!("test-{}", replica_id))
        .execute(&self.db)
        .await
        .expect("insert replica");

        deployment_id
    }

    /// Send a JSON-RPC 2.0 request to the A2A endpoint for the given agent.
    pub async fn a2a_rpc(
        &self,
        agent_name: &str,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let url = format!("{}/a2a/{agent_name}", self.gw_url());
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": Uuid::new_v4().to_string(),
            "method": method,
            "params": params,
        });

        self.client
            .post(&url)
            .json(&body)
            .send()
            .await
            .expect("a2a rpc request failed")
            .json()
            .await
            .expect("a2a rpc response parse failed")
    }

    /// GET helper.
    pub async fn get_json(&self, path: &str) -> (u16, serde_json::Value) {
        let url = format!("{}{path}", self.gw_url());
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .expect("GET request failed");
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!(null));
        (status, body)
    }

    /// POST JSON helper.
    pub async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> (u16, serde_json::Value) {
        let url = format!("{}{path}", self.gw_url());
        let resp = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .expect("POST request failed");
        let status = resp.status().as_u16();
        let resp_body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!(null));
        (status, resp_body)
    }
}
