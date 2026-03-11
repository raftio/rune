use bollard::container::{
    Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::Docker;
use futures_util::StreamExt;

const DEFAULT_IMAGE: &str = "ubuntu:latest";
const MAX_OUTPUT_BYTES: usize = 100_000;

pub async fn docker_exec(input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    let container_id = input["container_id"].as_str();
    let image = input["image"].as_str().unwrap_or(DEFAULT_IMAGE);

    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| format!("Failed to connect to Docker daemon: {e}"))?;

    let ephemeral = container_id.is_none();
    let cid = if let Some(id) = container_id {
        id.to_string()
    } else {
        create_ephemeral(&docker, image).await?
    };

    let result = exec_in_container(&docker, &cid, command).await;

    if ephemeral {
        let _ = docker
            .remove_container(
                &cid,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
    }

    result
}

async fn create_ephemeral(docker: &Docker, image: &str) -> Result<String, String> {
    let config = Config {
        image: Some(image.to_string()),
        cmd: Some(vec!["sleep".into(), "300".into()]),
        tty: Some(false),
        ..Default::default()
    };

    let name = format!("rune-exec-{}", uuid::Uuid::new_v4());
    let container = docker
        .create_container(
            Some(CreateContainerOptions {
                name: name.as_str(),
                platform: None,
            }),
            config,
        )
        .await
        .map_err(|e| format!("Failed to create container: {e}"))?;

    docker
        .start_container(&container.id, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| format!("Failed to start container: {e}"))?;

    Ok(container.id)
}

async fn exec_in_container(
    docker: &Docker,
    container_id: &str,
    command: &str,
) -> Result<serde_json::Value, String> {
    let exec = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(vec!["sh", "-c", command]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| format!("Failed to create exec: {e}"))?;

    let output = docker
        .start_exec(&exec.id, None)
        .await
        .map_err(|e| format!("Failed to start exec: {e}"))?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    if let StartExecResults::Attached { mut output, .. } = output {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let next = tokio::time::timeout_at(deadline, output.next()).await;
            match next {
                Ok(Some(Ok(msg))) => {
                    let text = msg.to_string();
                    match msg {
                        bollard::container::LogOutput::StdOut { .. } => {
                            if stdout.len() < MAX_OUTPUT_BYTES {
                                stdout.push_str(&text);
                            }
                        }
                        bollard::container::LogOutput::StdErr { .. } => {
                            if stderr.len() < MAX_OUTPUT_BYTES {
                                stderr.push_str(&text);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Some(Err(e))) => {
                    stderr.push_str(&format!("\n[stream error: {e}]"));
                    break;
                }
                Ok(None) => break,
                Err(_) => {
                    stderr.push_str("\n[timeout: 30s exceeded]");
                    break;
                }
            }
        }
    }

    let inspect = docker
        .inspect_exec(&exec.id)
        .await
        .map_err(|e| format!("Failed to inspect exec: {e}"))?;
    let exit_code = inspect.exit_code.unwrap_or(-1);

    Ok(serde_json::json!({
        "exit_code": exit_code,
        "stdout": stdout.trim_end(),
        "stderr": stderr.trim_end(),
        "container_id": container_id,
    }))
}
