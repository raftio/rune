use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use cron::Schedule;
use rune_storage::{RuneStore, RuntimeStore};
use tracing::{error, info};

use crate::error::RuntimeError;

fn to_cron_7_field(expr: &str) -> String {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() == 7 {
        return expr.to_string();
    }
    if parts.len() == 5 {
        return format!("0 {} *", expr);
    }
    expr.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- to_cron_7_field ---

    #[test]
    fn five_field_expr_gets_seconds_and_year_prepended_appended() {
        // 5-field: "MIN HOUR DOM MON DOW" → "0 MIN HOUR DOM MON DOW *"
        let out = to_cron_7_field("0 9 * * MON-FRI");
        let parts: Vec<&str> = out.split_whitespace().collect();
        assert_eq!(parts.len(), 7);
        // First field is seconds (0), last is year (*)
        assert_eq!(parts[0], "0");
        assert_eq!(parts[6], "*");
        // Original 5 fields preserved in the middle
        assert_eq!(parts[1], "0");
        assert_eq!(parts[2], "9");
    }

    #[test]
    fn seven_field_expr_returned_unchanged() {
        let expr = "0 0 9 * * MON-FRI *";
        assert_eq!(to_cron_7_field(expr), expr);
    }

    #[test]
    fn other_length_expr_returned_unchanged() {
        let expr = "0 9 *";
        assert_eq!(to_cron_7_field(expr), expr);
    }

    #[test]
    fn five_field_every_minute() {
        // "* * * * *" → "0 * * * * * *"
        let out = to_cron_7_field("* * * * *");
        let parts: Vec<&str> = out.split_whitespace().collect();
        assert_eq!(parts.len(), 7);
        assert_eq!(parts[0], "0");
    }

    #[test]
    fn five_field_preserves_all_original_fields_in_order() {
        let out = to_cron_7_field("30 6 1 1 SUN");
        // Expected: "0 30 6 1 1 SUN *"
        let parts: Vec<&str> = out.split_whitespace().collect();
        assert_eq!(parts[0], "0");
        assert_eq!(parts[1], "30");
        assert_eq!(parts[2], "6");
        assert_eq!(parts[3], "1");
        assert_eq!(parts[4], "1");
        assert_eq!(parts[5], "SUN");
        assert_eq!(parts[6], "*");
    }
}

const MAX_BACKOFF_SECS: u64 = 300; // 5 minutes

pub struct SchedulerWorker {
    store: Arc<RuneStore>,
    interval: Duration,
}

impl SchedulerWorker {
    pub fn new(store: Arc<RuneStore>) -> Self {
        Self {
            store,
            interval: Duration::from_secs(60),
        }
    }

    pub async fn run(self) {
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut consecutive_failures: u32 = 0;

        loop {
            ticker.tick().await;
            match self.tick().await {
                Ok(()) => {
                    if consecutive_failures > 0 {
                        info!(
                            consecutive_failures,
                            "scheduler recovered after consecutive failures"
                        );
                        consecutive_failures = 0;
                    }
                }
                Err(e) => {
                    consecutive_failures += 1;
                    // Exponential backoff: 2^n seconds, capped at MAX_BACKOFF_SECS.
                    let backoff_secs = std::cmp::min(
                        2u64.saturating_pow(consecutive_failures.min(8)),
                        MAX_BACKOFF_SECS,
                    );
                    error!(
                        error = %e,
                        consecutive_failures,
                        backoff_secs,
                        "scheduler tick failed — backing off before retry"
                    );
                    tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                }
            }
        }
    }

    async fn tick(&self) -> Result<(), RuntimeError> {
        let now = chrono::Utc::now();
        let now_str = now.to_rfc3339();

        let rows = self.store.list_enabled_schedules().await?;

        for row in rows {
            let Some(ref cron_expr) = row.cron_expr else {
                continue;
            };

            let expr_7 = to_cron_7_field(cron_expr);
            let schedule = match Schedule::from_str(&expr_7) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(schedule_id = %row.id, cron = %cron_expr, "invalid cron: {e}");
                    continue;
                }
            };

            let last_run = self.store.get_schedule_last_run(&row.id).await?;
            let after_ts = last_run
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or(now - chrono::Duration::days(1));

            let next = schedule
                .after(&after_ts)
                .next()
                .map(|dt| dt.with_timezone(&chrono::Utc));

            let is_due = next.map_or(false, |n| n <= now + chrono::Duration::seconds(30));

            if is_due {
                let agent = row.agent.as_deref().unwrap_or("default");
                let description = row.description.as_deref().unwrap_or("scheduled task");
                info!(schedule_id = %row.id, agent, "triggering scheduled invocation");
                if let Err(e) = self
                    .trigger_invocation(
                        &row.id,
                        agent,
                        description,
                        &now_str,
                        row.channel_type.as_deref(),
                        row.channel_recipient.as_deref(),
                    )
                    .await
                {
                    error!(schedule_id = %row.id, error = %e, "failed to trigger invocation");
                }
            }

            if let Some(next_dt) = schedule.after(&now).next() {
                let next_str = next_dt.with_timezone(&chrono::Utc).to_rfc3339();
                self.store.update_schedule_next_run(&row.id, &next_str).await?;
            }
        }

        Ok(())
    }

    async fn trigger_invocation(
        &self,
        schedule_id: &str,
        agent_name: &str,
        description: &str,
        now: &str,
        channel_type: Option<&str>,
        channel_recipient: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.store.update_schedule_last_run(schedule_id, now).await?;

        let deployment_id = self.store.resolve_deployment_for_agent(agent_name).await?;

        let Some(deployment_id) = deployment_id else {
            tracing::warn!(agent = agent_name, "no active deployment for scheduled agent");
            return Ok(());
        };

        let session_id = self.store.create_session(deployment_id, None, None).await?;

        let request_id = uuid::Uuid::new_v4();
        let mut trigger_msg = format!("[SCHEDULED] {} (schedule_id: {})", description, schedule_id);
        if let (Some(ch), Some(rec)) = (channel_type, channel_recipient) {
            trigger_msg.push_str(&format!(
                "\n[CHANNEL_CONTEXT] channel={ch} recipient={rec}\nSend your report to this channel/recipient using rune@channel-send."
            ));
        }
        let payload = serde_json::json!(trigger_msg);
        self.store
            .insert_request(request_id, session_id, deployment_id, None, &payload)
            .await?;

        info!(
            schedule_id,
            agent = agent_name,
            request_id = %request_id,
            "scheduled invocation queued"
        );
        Ok(())
    }
}
