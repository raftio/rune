use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: serde_json::Value,
    pub step: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointBlob {
    pub messages: Vec<SessionMessage>,
    pub step: i64,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// A2A task types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aTaskRow {
    pub task_id: String,
    pub context_id: String,
    pub request_id: String,
    pub session_id: String,
    pub agent_name: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub created_at: String,
}
