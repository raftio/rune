use clap::{Parser, Subcommand, ValueEnum};

fn default_log_file() -> String {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = home.join(".rune");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("rune.log").to_string_lossy().into_owned()
}

fn default_database_url() -> String {
    if let Some(from_env) = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()) {
        return from_env;
    }
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = home.join(".rune");
    let _ = std::fs::create_dir_all(&dir);
    format!("sqlite:{}", dir.join("rune.db").display())
}

#[derive(Parser)]
#[command(name = "rune", about = "Rune — Agent Runtime CLI", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage deployed agents
    Agent {
        #[command(subcommand)]
        cmd: AgentCommand,
    },
    /// Deploy an agent from a local spec or git source
    Run(RunArgs),
    /// List active deployments and their replica health
    Status(StatusArgs),
    /// View conversation session history
    Sessions(SessionsArgs),
    /// Start or stop the Rune server daemon
    #[command(alias = "deamon")]
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCommand,
    },
    /// Stop a deployment (scale to 0 replicas)
    Stop(StopAgentArgs),
    /// Remove a deployment
    Rm(RmAgentArgs),
    /// Deploy and manage multiple agents from a compose file
    Compose {
        #[command(subcommand)]
        cmd: ComposeCommand,
    },
    /// Manage Raft cluster membership
    Cluster {
        #[command(subcommand)]
        cmd: ClusterCommand,
    },
    /// Tail daemon logs, or restart daemon with a new log level
    Debug(DebugArgs),
}

#[derive(Subcommand)]
pub enum AgentCommand {
    /// List deployed agents
    Ls(AgentLsArgs),
    /// Show deployment details for an agent
    Inspect(AgentInspectArgs),
    /// View conversation session history for an agent
    Sessions(AgentSessionsArgs),
    /// Stop a deployed agent by name (scale to 0)
    Stop(AgentStopArgs),
    /// Remove a deployed agent by name
    Rm(AgentRmArgs),
}

#[derive(clap::Args)]
pub struct AgentInspectArgs {
    /// Agent name
    pub name: String,
    /// Filter by namespace
    #[arg(long)]
    pub ns: Option<String>,
    /// Filter by rollout alias
    #[arg(long)]
    pub alias: Option<String>,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct AgentSessionsArgs {
    /// Agent name
    pub name: String,
    /// Filter by namespace
    #[arg(long)]
    pub ns: Option<String>,
    /// Filter by rollout alias
    #[arg(long)]
    pub alias: Option<String>,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
    #[arg(long, default_value_t = default_database_url())]
    pub database_url: String,
}

#[derive(clap::Args)]
pub struct AgentLsArgs {
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct AgentStopArgs {
    /// Agent name
    pub name: String,
    /// Filter by namespace
    #[arg(long)]
    pub ns: Option<String>,
    /// Filter by rollout alias
    #[arg(long)]
    pub alias: Option<String>,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct AgentRmArgs {
    /// Agent name
    pub name: String,
    /// Filter by namespace
    #[arg(long)]
    pub ns: Option<String>,
    /// Filter by rollout alias
    #[arg(long)]
    pub alias: Option<String>,
    /// Force delete even when cascade fails
    #[arg(long)]
    pub force: bool,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct RunArgs {
    /// Agent source: path to Runefile, agent directory containing a Runefile, or git://repo-url[#subdir]
    pub agent_spec: String,
    #[arg(long, default_value = "dev")]
    pub namespace: String,
    #[arg(long, default_value = "stable")]
    pub alias: String,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct StatusArgs {
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct SessionsArgs {
    /// Session ID or deployment ID (use deployment ID to view latest session)
    pub id: String,
    #[arg(long, default_value_t = default_database_url())]
    pub database_url: String,
}

#[derive(clap::Args)]
pub struct StartArgs {
    #[arg(long, default_value = "0.0.0.0:8080")]
    pub gateway_addr: String,
    #[arg(long, default_value = "0.0.0.0:8081")]
    pub control_plane_addr: String,
    #[arg(long, default_value_t = default_database_url())]
    pub database_url: String,
    #[arg(long, default_value = "/tmp/rune.pid")]
    pub pid_file: String,
    /// Run in foreground instead of daemonizing
    #[arg(short, long)]
    pub foreground: bool,
    /// Log file path for daemon stdout/stderr
    #[arg(long, default_value_t = default_log_file())]
    pub log_file: String,

    /// Node ID for Raft cluster mode (omit for standalone)
    #[arg(long)]
    pub node_id: Option<u64>,
    /// Raft gRPC listen address (required when --node-id is set)
    #[arg(long, default_value = "0.0.0.0:9000")]
    pub raft_addr: String,
    /// Peer nodes in the form ID@HOST:PORT (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub peers: Vec<String>,

    /// Execution backend for agent replicas
    #[arg(long, default_value = "wasm")]
    pub backend: BackendChoice,
    /// Kubernetes namespace (only used when --backend=kubernetes)
    #[arg(long, default_value = "rune")]
    pub k8s_namespace: String,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum BackendChoice {
    Wasm,
    Docker,
    Kubernetes,
}

#[derive(clap::Args)]
pub struct StopArgs {
    #[arg(long, default_value = "/tmp/rune.pid")]
    pub pid_file: String,
}

#[derive(clap::Args)]
pub struct StopAgentArgs {
    pub deployment_id: String,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct RmAgentArgs {
    pub deployment_id: String,
    /// Force delete even when cascade fails (disables foreign key checks)
    #[arg(long)]
    pub force: bool,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(Subcommand)]
pub enum DaemonCommand {
    /// Launch the gateway and control-plane servers
    Start(StartArgs),
    /// Gracefully shut down a running Rune server
    Stop(StopArgs),
    /// Show whether the Rune daemon is running
    Status(DaemonStatusArgs),
}

#[derive(clap::Args)]
pub struct DebugArgs {
    /// Restart daemon before tailing logs
    #[arg(long)]
    pub restart: bool,
    /// Log level to use when restarting (trace | debug | info | warn | error)
    #[arg(long, default_value = "info")]
    pub log_level: String,
    /// PID file path (must match what daemon was started with)
    #[arg(long, default_value = "/tmp/rune.pid")]
    pub pid_file: String,
    /// Log file to tail
    #[arg(long, default_value_t = default_log_file())]
    pub log_file: String,
}

#[derive(clap::Args)]
pub struct DaemonStatusArgs {
    #[arg(long, default_value = "/tmp/rune.pid")]
    pub pid_file: String,
}

#[derive(Subcommand)]
pub enum ComposeCommand {
    /// Deploy all agents defined in the compose file
    Up(ComposeUpArgs),
    /// Tear down all agents in the compose project
    Down(ComposeDownArgs),
    /// List status of agents in the compose project
    Ps(ComposePsArgs),
}

#[derive(clap::Args)]
pub struct ComposeUpArgs {
    /// Path to the compose file
    #[arg(short, long, default_value = "rune-compose.yaml")]
    pub file: String,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct ComposeDownArgs {
    /// Path to the compose file
    #[arg(short, long, default_value = "rune-compose.yaml")]
    pub file: String,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(clap::Args)]
pub struct ComposePsArgs {
    /// Path to the compose file
    #[arg(short, long, default_value = "rune-compose.yaml")]
    pub file: String,
    #[arg(long, default_value = "http://localhost:8081")]
    pub control_plane: String,
}

#[derive(Subcommand)]
pub enum ClusterCommand {
    /// Initialize a single-node Raft cluster
    Init(ClusterInitArgs),
    /// Add a learner node to the cluster
    AddLearner(ClusterAddLearnerArgs),
    /// Change voting membership of the cluster
    ChangeMembership(ClusterChangeMembershipArgs),
    /// Show cluster status (leader, members, log index)
    Status(ClusterStatusArgs),
}

#[derive(clap::Args)]
pub struct ClusterInitArgs {
    /// Address of any existing node in the cluster
    #[arg(long, default_value = "http://localhost:9000")]
    pub addr: String,
}

#[derive(clap::Args)]
pub struct ClusterAddLearnerArgs {
    /// Node ID to add
    pub node_id: u64,
    /// Address of the node to add (HOST:PORT)
    pub node_addr: String,
    /// Address of the leader node for the Raft admin call
    #[arg(long, default_value = "http://localhost:9000")]
    pub leader_addr: String,
}

#[derive(clap::Args)]
pub struct ClusterChangeMembershipArgs {
    /// Comma-separated list of voter node IDs
    #[arg(value_delimiter = ',')]
    pub members: Vec<u64>,
    /// Address of the leader node for the Raft admin call
    #[arg(long, default_value = "http://localhost:9000")]
    pub leader_addr: String,
}

#[derive(clap::Args)]
pub struct ClusterStatusArgs {
    /// Address of any node in the cluster
    #[arg(long, default_value = "http://localhost:9000")]
    pub addr: String,
}
