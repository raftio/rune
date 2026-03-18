use crate::BuiltinToolDef;
use serde_json::json;

pub fn all() -> Vec<BuiltinToolDef> {
    vec![
        // ── Memory ──────────────────────────────────────────────────
        BuiltinToolDef {
            name: "rune@memory-store",
            description: "Persist a value in shared key-value memory across sessions and restarts. \
                Use consistent, descriptive keys such as 'user_name', 'user_preferences', \
                'user_language', 'last_topic', or 'onboarding_done'. \
                Call this whenever the user shares personal details or preferences worth remembering.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Descriptive storage key, e.g. 'user_name', 'user_preferences'" },
                    "value": { "type": "string", "description": "The value to store (JSON-encode objects/arrays)" }
                },
                "required": ["key", "value"]
            }),
        },
        BuiltinToolDef {
            name: "rune@memory-recall",
            description: "Recall a previously stored value from shared memory by exact key. \
                Call this at the start of a conversation or when the user references something \
                they shared before. Common keys: 'user_name', 'user_preferences', 'user_language', \
                'last_topic'. If unsure what keys exist, use rune@memory-list first.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The exact storage key to recall" }
                },
                "required": ["key"]
            }),
        },
        BuiltinToolDef {
            name: "rune@memory-list",
            description: "List all keys currently stored in shared memory. \
                Use this to discover what has been remembered before calling rune@memory-recall.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
    ]
}
