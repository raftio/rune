//! Prometheus metrics for the Rune agent runtime.
//!
//! Metrics are recorded via helper functions. Uses the `metrics` crate with
//! `metrics-exporter-prometheus` for exposition.

/// Record request duration in seconds.
pub fn record_request_duration(agent: &str, status: &str, duration_secs: f64) {
    let labels = [
        ("agent", agent.to_string()),
        ("status", status.to_string()),
    ];
    metrics::histogram!("rune_request_duration_seconds", &labels).record(duration_secs);
}

/// Record first token latency in seconds.
pub fn record_first_token_latency(secs: f64) {
    metrics::histogram!("rune_first_token_latency_seconds").record(secs);
}

/// Record tool call duration in seconds.
pub fn record_tool_call_duration(tool: &str, status: &str, duration_secs: f64) {
    let labels = [
        ("tool", tool.to_string()),
        ("status", status.to_string()),
    ];
    metrics::histogram!("rune_tool_call_duration_seconds", &labels).record(duration_secs);
}

/// Record model/LLM call duration in seconds.
pub fn record_model_call_duration(provider: &str, model: &str, duration_secs: f64) {
    let labels = [
        ("provider", provider.to_string()),
        ("model", model.to_string()),
    ];
    metrics::histogram!("rune_model_call_duration_seconds", &labels).record(duration_secs);
}

/// Set active sessions gauge.
pub fn set_active_sessions(count: f64) {
    metrics::gauge!("rune_active_sessions").set(count);
}

/// Set replica load gauge.
pub fn set_replica_load(replica_id: &str, load: f64) {
    let labels = [("replica_id", replica_id.to_string())];
    metrics::gauge!("rune_replica_load", &labels).set(load);
}

/// Increment policy denials counter.
pub fn increment_policy_denials() {
    metrics::counter!("rune_policy_denials_total").increment(1);
}

/// Increment streaming events counter.
pub fn increment_streaming_events() {
    metrics::counter!("rune_streaming_events_total").increment(1);
}
