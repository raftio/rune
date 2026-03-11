use sqlx::SqlitePool;

use crate::error::StorageError;

struct Migration {
    version: i32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "agent_versions",
        sql: r#"
CREATE TABLE IF NOT EXISTS agent_versions (
    id             TEXT    PRIMARY KEY,
    agent_name     TEXT    NOT NULL,
    version        TEXT    NOT NULL,
    image_ref      TEXT    NOT NULL,
    image_digest   TEXT    NOT NULL,
    spec_sha256    TEXT    NOT NULL,
    signature_ref  TEXT,
    runtime_class  TEXT    NOT NULL,
    status         TEXT    NOT NULL DEFAULT 'pending',
    created_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (agent_name, version)
);
"#,
    },
    Migration {
        version: 2,
        name: "agent_deployments",
        sql: r#"
CREATE TABLE IF NOT EXISTS agent_deployments (
    id                 TEXT     PRIMARY KEY,
    agent_version_id   TEXT     NOT NULL REFERENCES agent_versions(id),
    environment        TEXT     NOT NULL,
    rollout_alias      TEXT     NOT NULL,
    desired_replicas   INTEGER  NOT NULL DEFAULT 1,
    min_replicas       INTEGER  NOT NULL DEFAULT 1,
    max_replicas       INTEGER  NOT NULL DEFAULT 1,
    concurrency_limit  INTEGER  NOT NULL DEFAULT 10,
    routing_policy     TEXT     NOT NULL DEFAULT '{}',
    resource_profile   TEXT     NOT NULL DEFAULT '{}',
    config_ref         TEXT,
    secret_ref         TEXT,
    status             TEXT     NOT NULL DEFAULT 'pending',
    created_at         TEXT     NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at         TEXT     NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#,
    },
    Migration {
        version: 3,
        name: "agent_replicas",
        sql: r#"
CREATE TABLE IF NOT EXISTS agent_replicas (
    id                   TEXT     PRIMARY KEY,
    deployment_id        TEXT     NOT NULL REFERENCES agent_deployments(id),
    backend_type         TEXT     NOT NULL,
    backend_instance_id  TEXT     NOT NULL,
    node_name            TEXT,
    endpoint             TEXT,
    state                TEXT     NOT NULL DEFAULT 'starting',
    concurrency_limit    INTEGER  NOT NULL DEFAULT 10,
    current_load         INTEGER  NOT NULL DEFAULT 0,
    started_at           TEXT,
    last_heartbeat_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_replicas_deployment_id ON agent_replicas(deployment_id);
CREATE INDEX IF NOT EXISTS idx_agent_replicas_state         ON agent_replicas(state);
"#,
    },
    Migration {
        version: 4,
        name: "agent_sessions",
        sql: r#"
CREATE TABLE IF NOT EXISTS agent_sessions (
    id                  TEXT    PRIMARY KEY,
    deployment_id       TEXT    NOT NULL REFERENCES agent_deployments(id),
    tenant_id           TEXT,
    routing_key         TEXT,
    assigned_replica_id TEXT    REFERENCES agent_replicas(id),
    state_ref           TEXT,
    status              TEXT    NOT NULL DEFAULT 'active',
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_agent_sessions_deployment_id ON agent_sessions(deployment_id);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_replica_id    ON agent_sessions(assigned_replica_id);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_routing_key   ON agent_sessions(routing_key);
"#,
    },
    Migration {
        version: 5,
        name: "agent_requests",
        sql: r#"
CREATE TABLE IF NOT EXISTS agent_requests (
    id               TEXT    PRIMARY KEY,
    session_id       TEXT    REFERENCES agent_sessions(id),
    deployment_id    TEXT    NOT NULL REFERENCES agent_deployments(id),
    replica_id       TEXT    REFERENCES agent_replicas(id),
    request_payload  TEXT    NOT NULL DEFAULT '{}',
    response_summary TEXT,
    status           TEXT    NOT NULL DEFAULT 'pending',
    started_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at     TEXT,
    latency_ms       INTEGER,
    error_code       TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_requests_session_id    ON agent_requests(session_id);
CREATE INDEX IF NOT EXISTS idx_agent_requests_deployment_id ON agent_requests(deployment_id);
CREATE INDEX IF NOT EXISTS idx_agent_requests_started_at    ON agent_requests(started_at DESC);
"#,
    },
    Migration {
        version: 6,
        name: "tool_invocations",
        sql: r#"
CREATE TABLE IF NOT EXISTS tool_invocations (
    id              TEXT    PRIMARY KEY,
    request_id      TEXT    NOT NULL REFERENCES agent_requests(id),
    tool_name       TEXT    NOT NULL,
    tool_runtime    TEXT    NOT NULL,
    capability_set  TEXT    NOT NULL DEFAULT '[]',
    input_payload   TEXT    NOT NULL DEFAULT '{}',
    output_payload  TEXT,
    status          TEXT    NOT NULL DEFAULT 'pending',
    started_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at    TEXT,
    error_code      TEXT
);

CREATE INDEX IF NOT EXISTS idx_tool_invocations_request_id ON tool_invocations(request_id);
CREATE INDEX IF NOT EXISTS idx_tool_invocations_tool_name  ON tool_invocations(tool_name);
"#,
    },
    Migration {
        version: 7,
        name: "session_messages",
        sql: r#"
CREATE TABLE IF NOT EXISTS session_messages (
    id         TEXT    PRIMARY KEY,
    session_id TEXT    NOT NULL REFERENCES agent_sessions(id),
    role       TEXT    NOT NULL,
    content    TEXT    NOT NULL DEFAULT '{}',
    step       INTEGER NOT NULL DEFAULT 0,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_session_messages_session_id ON session_messages(session_id);
"#,
    },
    Migration {
        version: 8,
        name: "a2a_tasks",
        sql: r#"
CREATE TABLE IF NOT EXISTS a2a_tasks (
    task_id       TEXT PRIMARY KEY,
    context_id    TEXT NOT NULL,
    request_id    TEXT NOT NULL REFERENCES agent_requests(id),
    session_id    TEXT NOT NULL REFERENCES agent_sessions(id),
    agent_name    TEXT NOT NULL,
    state         TEXT NOT NULL DEFAULT 'submitted',
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_a2a_tasks_context ON a2a_tasks(context_id);
CREATE INDEX IF NOT EXISTS idx_a2a_tasks_agent   ON a2a_tasks(agent_name);
CREATE INDEX IF NOT EXISTS idx_a2a_tasks_request ON a2a_tasks(request_id);
"#,
    },
    Migration {
        version: 9,
        name: "agent_calls",
        sql: r#"
CREATE TABLE IF NOT EXISTS agent_calls (
    id              TEXT PRIMARY KEY,
    caller_task_id  TEXT NOT NULL,
    callee_task_id  TEXT,
    caller_agent    TEXT NOT NULL,
    callee_agent    TEXT NOT NULL,
    callee_endpoint TEXT NOT NULL,
    depth           INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'pending',
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at    TEXT,
    latency_ms      INTEGER
);

CREATE INDEX IF NOT EXISTS idx_agent_calls_caller ON agent_calls(caller_task_id);
CREATE INDEX IF NOT EXISTS idx_agent_calls_callee ON agent_calls(callee_task_id);
CREATE INDEX IF NOT EXISTS idx_agent_calls_status ON agent_calls(status);
"#,
    },
    Migration {
        version: 10,
        name: "add_project_ref_to_deployments",
        sql: r#"
ALTER TABLE agent_deployments ADD COLUMN project_ref TEXT;
CREATE INDEX IF NOT EXISTS idx_agent_deployments_project_ref ON agent_deployments(project_ref);
"#,
    },
    Migration {
        version: 11,
        name: "kv_kg_tasks_events_schedules",
        sql: r#"
CREATE TABLE IF NOT EXISTS rune_kv (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS kg_entities (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    properties  TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS kg_relations (
    id         TEXT PRIMARY KEY,
    source     TEXT NOT NULL,
    relation   TEXT NOT NULL,
    target     TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 1.0,
    properties TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_kg_relations_source ON kg_relations(source);
CREATE INDEX IF NOT EXISTS idx_kg_relations_target ON kg_relations(target);

CREATE TABLE IF NOT EXISTS task_queue (
    id           TEXT PRIMARY KEY,
    title        TEXT NOT NULL,
    description  TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    assigned_to  TEXT,
    claimed_by   TEXT,
    result       TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_task_queue_status ON task_queue(status);

CREATE TABLE IF NOT EXISTS events (
    id         TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    payload    TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type);

CREATE TABLE IF NOT EXISTS schedules (
    id             TEXT PRIMARY KEY,
    description    TEXT,
    schedule_input TEXT,
    cron_expr      TEXT,
    agent          TEXT,
    enabled        INTEGER NOT NULL DEFAULT 1,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#,
    },
    Migration {
        version: 12,
        name: "policy_audit",
        sql: r#"
CREATE TABLE IF NOT EXISTS policy_audit (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    resource    TEXT NOT NULL,
    allowed     INTEGER NOT NULL,
    reason      TEXT,
    request_id  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_policy_audit_kind ON policy_audit(kind);
CREATE INDEX IF NOT EXISTS idx_policy_audit_request_id ON policy_audit(request_id);
"#,
    },
    Migration {
        version: 13,
        name: "session_checkpoints",
        sql: r#"
CREATE TABLE IF NOT EXISTS session_checkpoints (
    session_id   TEXT    PRIMARY KEY REFERENCES agent_sessions(id),
    data         TEXT    NOT NULL,
    step         INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_session_checkpoints_session_id ON session_checkpoints(session_id);
"#,
    },
    Migration {
        version: 14,
        name: "rune_cache",
        sql: r#"
CREATE TABLE IF NOT EXISTS rune_cache (
    namespace   TEXT    NOT NULL,
    key         TEXT    NOT NULL,
    value       TEXT    NOT NULL,
    expires_at  TEXT    NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (namespace, key)
);
CREATE INDEX IF NOT EXISTS idx_rune_cache_namespace_key ON rune_cache(namespace, key);
CREATE INDEX IF NOT EXISTS idx_rune_cache_expires ON rune_cache(expires_at);

CREATE TABLE IF NOT EXISTS rune_rate_limits (
    key           TEXT    NOT NULL PRIMARY KEY,
    window_start  TEXT    NOT NULL,
    count         INTEGER NOT NULL DEFAULT 0,
    window_seconds INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rune_rate_limits_key ON rune_rate_limits(key);
"#,
    },
    Migration {
        version: 15,
        name: "session_checkpoints_storage_ref",
        sql: "ALTER TABLE session_checkpoints ADD COLUMN storage_ref TEXT;",
    },
    Migration {
        version: 16,
        name: "schedules_last_next_run",
        sql: r#"
ALTER TABLE schedules ADD COLUMN last_run_at TEXT;
ALTER TABLE schedules ADD COLUMN next_run_at TEXT;
CREATE INDEX IF NOT EXISTS idx_schedules_next_run ON schedules(next_run_at) WHERE enabled = 1;
"#,
    },
    Migration {
        version: 17,
        name: "agent_deployment_networks",
        sql: r#"
ALTER TABLE agent_deployments ADD COLUMN networks TEXT NOT NULL DEFAULT '["bridge"]';
"#,
    },
    Migration {
        version: 18,
        name: "rename_environment_to_namespace_add_env_json",
        sql: r#"
ALTER TABLE agent_deployments RENAME COLUMN environment TO namespace;
ALTER TABLE agent_deployments ADD COLUMN env_json TEXT;
"#,
    },
    Migration {
        version: 19,
        name: "unique_deployment_ns_alias",
        sql: r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_deployments_ns_alias
    ON agent_deployments(namespace, rollout_alias);
"#,
    },
    Migration {
        version: 20,
        name: "add_agent_name_to_deployments",
        sql: r#"
ALTER TABLE agent_deployments ADD COLUMN agent_name TEXT NOT NULL DEFAULT '';
UPDATE agent_deployments SET agent_name = (
    SELECT av.agent_name FROM agent_versions av WHERE av.id = agent_deployments.agent_version_id
);
DROP INDEX IF EXISTS idx_agent_deployments_ns_alias;
CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_deployments_ns_alias
    ON agent_deployments(namespace, rollout_alias, agent_name);
"#,
    },
    Migration {
        version: 21,
        name: "schedules_channel_context",
        sql: r#"
ALTER TABLE schedules ADD COLUMN channel_type TEXT;
ALTER TABLE schedules ADD COLUMN channel_recipient TEXT;
"#,
    },
];

pub async fn run_migrations(db: &SqlitePool) -> Result<(), StorageError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version    INTEGER PRIMARY KEY,
            name       TEXT    NOT NULL,
            applied_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        )",
    )
    .execute(db)
    .await?;

    for m in MIGRATIONS {
        let applied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM _migrations WHERE version = ?)",
        )
        .bind(m.version)
        .fetch_one(db)
        .await?;

        if applied {
            continue;
        }

        tracing::debug!(version = m.version, name = m.name, "applying migration");

        for statement in m.sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(statement).execute(db).await?;
        }

        sqlx::query("INSERT INTO _migrations (version, name) VALUES (?, ?)")
            .bind(m.version)
            .bind(m.name)
            .execute(db)
            .await?;
    }

    Ok(())
}
