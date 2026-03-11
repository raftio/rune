use k8s_openapi::api::core::v1::Pod;
use kube::api::ListParams;
use kube::Api;
use std::time::Duration;

use rune_runtime::error::RuntimeError;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Wait until at least one Pod matching the given label selector becomes Ready.
pub async fn wait_for_pod_ready(
    pods_api: &Api<Pod>,
    label_selector: &str,
    timeout: Option<Duration>,
) -> Result<String, RuntimeError> {
    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let lp = ListParams::default().labels(label_selector);
        let pod_list = pods_api
            .list(&lp)
            .await
            .map_err(|e| RuntimeError::Backend(format!("list pods: {e}")))?;

        for pod in &pod_list.items {
            if is_pod_ready(pod) {
                let name = pod
                    .metadata
                    .name
                    .clone()
                    .unwrap_or_else(|| "unknown".into());
                return Ok(name);
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(RuntimeError::Backend(format!(
                "timeout ({timeout:?}) waiting for pod with selector '{label_selector}' to become ready"
            )));
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn is_pod_ready(pod: &Pod) -> bool {
    let Some(status) = &pod.status else {
        return false;
    };
    let Some(conditions) = &status.conditions else {
        return false;
    };
    conditions
        .iter()
        .any(|c| c.type_ == "Ready" && c.status == "True")
}

/// Check if a named Pod is in Running phase with Ready condition.
pub fn check_pod_healthy(pod: &Pod) -> (bool, Option<String>) {
    let Some(status) = &pod.status else {
        return (false, Some("no status".into()));
    };

    let phase = status.phase.as_deref().unwrap_or("Unknown");
    if phase != "Running" {
        return (false, Some(format!("phase: {phase}")));
    }

    let ready = status
        .conditions
        .as_ref()
        .map(|cs| cs.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
        .unwrap_or(false);

    if ready {
        (true, None)
    } else {
        (false, Some("running but not ready".into()))
    }
}
