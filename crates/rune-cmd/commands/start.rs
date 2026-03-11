use anyhow::{bail, Result};
use std::sync::Arc;

use rune_env::PlatformEnv;
use rune_storage::{PoolConfig, RuneStore};
use rune_storage::raft::{
    RaftGrpcServer, RuneNetworkFactory, RuneRaftStore, RuneTypeConfig,
};

use crate::cli::StartArgs;

pub async fn exec(args: StartArgs, platform_env: PlatformEnv) -> Result<()> {
    if !args.foreground {
        return daemonize(&args);
    }

    std::fs::write(&args.pid_file, std::process::id().to_string())?;

    let result = run_server(&args, platform_env).await;

    let _ = std::fs::remove_file(&args.pid_file);
    result
}

fn daemonize(args: &StartArgs) -> Result<()> {
    let exe = std::env::current_exe()?;

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon").arg("start").arg("--foreground");
    cmd.arg("--gateway-addr").arg(&args.gateway_addr);
    cmd.arg("--control-plane-addr").arg(&args.control_plane_addr);
    cmd.arg("--database-url").arg(&args.database_url);
    cmd.arg("--pid-file").arg(&args.pid_file);

    let backend_str = match args.backend {
        crate::cli::BackendChoice::Wasm => "wasm",
        crate::cli::BackendChoice::Docker => "docker",
        crate::cli::BackendChoice::Kubernetes => "kubernetes",
    };
    cmd.arg("--backend").arg(backend_str);
    cmd.arg("--k8s-namespace").arg(&args.k8s_namespace);

    cmd.arg("--log-file").arg(&args.log_file);

    cmd.stdin(std::process::Stdio::null());

    // Rotate log: rename existing rune.log -> rune.log.1, then create fresh file
    let log_path = std::path::Path::new(&args.log_file);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if log_path.exists() {
        let rotated = format!("{}.1", args.log_file);
        let _ = std::fs::rename(log_path, &rotated);
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)?;
    cmd.stdout(log_file.try_clone()?);
    cmd.stderr(log_file);

    let child = cmd.spawn()?;
    let pid = child.id();

    println!();
    println!("  \x1b[32m▲\x1b[0m \x1b[1mRune started\x1b[0m");
    println!();
    println!("    \x1b[2mGateway\x1b[0m         {}", args.gateway_addr);
    println!("    \x1b[2mControl Plane\x1b[0m   {}", args.control_plane_addr);
    println!("    \x1b[2mPID\x1b[0m             {pid}");
    println!("    \x1b[2mPID file\x1b[0m        {}", args.pid_file);
    println!("    \x1b[2mLog file\x1b[0m        {}", args.log_file);
    println!();
    Ok(())
}

async fn run_server(args: &StartArgs, platform_env: PlatformEnv) -> Result<()> {
    let platform_env = Arc::new(platform_env);

    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| anyhow::anyhow!("metrics recorder: {e}"))?;

    let mut store = RuneStore::new(PoolConfig {
        database_url: args.database_url.clone(),
        max_connections: 4,
        create_if_missing: true,
    })
    .await?;

    tracing::info!("Running database migrations...");
    store.run_migrations().await?;
    tracing::info!("Migrations done.");

    // --- Optional Raft cluster mode ---
    if let Some(node_id) = args.node_id {
        tracing::info!(node_id, raft_addr = %args.raft_addr, "Starting Raft cluster mode");

        let raft_store = RuneRaftStore::new(store.pool().clone());

        let config = Arc::new(
            openraft::Config {
                heartbeat_interval: 500,
                election_timeout_min: 1500,
                election_timeout_max: 3000,
                ..Default::default()
            },
        );

        let (log_store, state_machine) = openraft::storage::Adaptor::new(raft_store);
        let network = RuneNetworkFactory;

        let raft = openraft::Raft::<RuneTypeConfig>::new(
            node_id,
            config,
            network,
            log_store,
            state_machine,
        )
        .await
        .map_err(|e| anyhow::anyhow!("failed to create Raft instance: {e}"))?;

        let raft_arc = Arc::new(raft.clone());

        let raft_addr: std::net::SocketAddr = args.raft_addr.parse()?;
        tokio::spawn({
            let raft_grpc = RaftGrpcServer::new(raft_arc);
            async move {
                tracing::info!("Raft gRPC → {raft_addr}");
                if let Err(e) = tonic::transport::Server::builder()
                    .add_service(raft_grpc.into_service())
                    .serve(raft_addr)
                    .await
                {
                    tracing::error!(error = %e, "Raft gRPC server failed");
                }
            }
        });

        store.enable_cluster(raft);
        tracing::info!("Raft cluster mode enabled.");
    }

    let store = Arc::new(store);

    let backend: Arc<dyn rune_runtime::RuntimeBackend> = match args.backend {
        crate::cli::BackendChoice::Wasm => {
            Arc::new(rune_wasm_backend::WasmBackend::new(store.clone(), platform_env.clone())?)
        }
        crate::cli::BackendChoice::Docker => {
            Arc::new(rune_docker_backend::DockerBackend::new(store.clone(), platform_env.clone())?)
        }
        crate::cli::BackendChoice::Kubernetes => {
            Arc::new(
                rune_k8s_backend::KubernetesBackend::new(
                    store.clone(),
                    args.k8s_namespace.clone(),
                    platform_env.clone(),
                )
                .await?,
            )
        }
    };

    // Leadership-gated workers
    tokio::spawn({
        let store = store.clone();
        let backend = backend.clone();
        async move {
            loop {
                store.wait_for_leadership().await;
                tracing::info!("This node is the leader — starting workers");

                let reconcile =
                    rune_runtime::ReconcileLoop::new(store.clone(), backend.clone());
                let scheduler =
                    rune_runtime::scheduler::SchedulerWorker::new(store.clone());
                let cache = rune_runtime::SqliteCache::new(store.clone());

                tokio::select! {
                    _ = reconcile.run() => {
                        tracing::warn!("reconcile loop exited unexpectedly");
                    }
                    _ = scheduler.run() => {
                        tracing::warn!("scheduler worker exited unexpectedly");
                    }
                    _ = async {
                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(60));
                        loop {
                            interval.tick().await;
                            if let Err(e) = cache.cleanup_expired().await {
                                tracing::warn!(error = %e, "cache cleanup failed");
                            }
                        }
                    } => {}
                    _ = store.wait_for_leadership_lost() => {
                        tracing::info!("Lost leadership — stopping workers");
                    }
                }
            }
        }
    });

    // --- Channel bridge setup ---
    let channel_bridge = Arc::new(rune_gateway::RuntimeChannelBridge::new(store.clone(), platform_env.clone()));
    let bridge_ref = Arc::new(rune_gateway::ChannelBridgeRef::new(channel_bridge.clone()));
    let channel_router = Arc::new(rune_channels::AgentRouter::new());

    let mut bridge_manager = rune_channels::BridgeManager::new(channel_bridge, channel_router);

    let channel_configs = load_channel_configs(&platform_env);
    let mut channels_attempted = 0u32;
    let mut channels_started = 0u32;
    for cfg in &channel_configs {
        channels_attempted += 1;
        match rune_channels::create_adapter(cfg) {
            Ok(adapter) => {
                let name = adapter.name().to_string();
                if let Err(e) = bridge_manager.start_adapter(adapter).await {
                    tracing::warn!(channel = %name, error = %e, "Failed to start channel adapter");
                } else {
                    channels_started += 1;
                    tracing::info!(channel = %name, "Channel adapter started");
                }
            }
            Err(e) => {
                tracing::warn!(channel = %cfg.channel_type, error = %e, "Failed to create channel adapter");
            }
        }
    }
    if channels_attempted > 0 && channels_started == 0 {
        tracing::error!(
            attempted = channels_attempted,
            "All channel adapters failed to start — channels will be unavailable"
        );
    }

    bridge_ref.update_statuses(bridge_manager.adapter_statuses());

    let gw_router = rune_gateway::router_with_bridge(
        store.clone(),
        Some(backend.clone()),
        Some(prometheus_handle),
        Some(bridge_ref),
        platform_env.clone(),
    );
    let cp_router = rune_cp::router(store.clone());

    let gw_addr: std::net::SocketAddr = args.gateway_addr.parse()?;
    let cp_addr: std::net::SocketAddr = args.control_plane_addr.parse()?;

    tracing::info!("Gateway       → {gw_addr}");
    tracing::info!("Control plane → {cp_addr}");

    let gw = tokio::net::TcpListener::bind(gw_addr).await?;
    let cp = tokio::net::TcpListener::bind(cp_addr).await?;

    tokio::try_join!(axum::serve(gw, gw_router), axum::serve(cp, cp_router))?;

    bridge_manager.stop().await;

    bail!("server exited unexpectedly");
}

/// Load channel adapter configs from `channels.yaml` (if present) or env vars.
fn load_channel_configs(env: &PlatformEnv) -> Vec<rune_channels::ChannelConfig> {
    let config_paths = [
        "channels.yaml",
        "channels.yml",
        "/etc/rune/channels.yaml",
    ];

    for path in &config_paths {
        if let Ok(contents) = std::fs::read_to_string(path) {
            match serde_yaml::from_str::<rune_channels::ChannelsFile>(&contents) {
                Ok(file) => {
                    tracing::info!(path = %path, count = file.channels.len(), "Loaded channel config");
                    return file.channels;
                }
                Err(e) => {
                    tracing::warn!(path = %path, error = %e, "Failed to parse channel config");
                }
            }
        }
    }

    if let Some(path) = &env.channels_config_path {
        if let Ok(contents) = std::fs::read_to_string(path) {
            match serde_yaml::from_str::<rune_channels::ChannelsFile>(&contents) {
                Ok(file) => {
                    tracing::info!(path = %path, count = file.channels.len(), "Loaded channel config");
                    return file.channels;
                }
                Err(e) => {
                    tracing::warn!(path = %path, error = %e, "Failed to parse channel config");
                }
            }
        }
    }

    let configs = rune_channels::configs_from_env(env);
    if !configs.is_empty() {
        tracing::info!(count = configs.len(), "Loaded channel config from env vars");
    }
    configs
}
