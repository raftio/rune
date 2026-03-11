mod cli;
mod commands;

use anyhow::Result;
use opentelemetry_otlp::WithExportConfig;
use rune_env::PlatformEnv;
use tracing_subscriber::filter::FilterFn;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

fn init_tracing(env: &PlatformEnv) -> Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info".into());

    // Suppress verbose HTTP client logs from hyper/hyper_util (TRACE/DEBUG)
    let suppress_hyper = FilterFn::new(|metadata| {
        let target = metadata.target();
        !target.starts_with("hyper_util") && !target.starts_with("hyper")
    });

    let fmt_layer: Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync> =
        if env.log_format_json {
            Box::new(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_span_list(true)
                    .with_filter(suppress_hyper),
            )
        } else {
            Box::new(
                tracing_subscriber::fmt::layer().with_filter(suppress_hyper),
            )
        };

    let mut layers: Vec<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>> =
        vec![env_filter.boxed(), fmt_layer];

    if let Some(endpoint) = &env.otel_endpoint {
        let ep = endpoint.trim_end_matches('/');
        if let Ok(exporter) = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(ep)
            .build()
        {
            let resource = opentelemetry_sdk::Resource::new(vec![
                opentelemetry::KeyValue::new("service.name", "rune"),
            ]);
            let provider = opentelemetry_sdk::trace::TracerProvider::builder()
                .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
                .with_resource(resource)
                .build();
            opentelemetry::global::set_tracer_provider(provider);
            layers.push(tracing_opentelemetry::layer().boxed());
        }
    }

    tracing_subscriber::registry()
        .with(layers)
        .init();
    Ok(())
}
use clap::Parser;

use cli::{AgentCommand, Cli, ClusterCommand, Command, ComposeCommand, DaemonCommand};


#[tokio::main]
async fn main() -> Result<()> {
    let platform_env = PlatformEnv::from_env()
        .map_err(|e| anyhow::anyhow!("invalid environment configuration: {e}"))?;

    init_tracing(&platform_env)?;
    let cli = Cli::parse();

    match cli.command {
        Command::Agent { cmd } => match cmd {
            AgentCommand::Ls(args) => commands::agent::ls(args).await?,
            AgentCommand::Inspect(args) => commands::agent::inspect(args).await?,
            AgentCommand::Sessions(args) => commands::agent::sessions_by_name(args).await?,
            AgentCommand::Stop(args) => commands::agent::stop_by_name(args).await?,
            AgentCommand::Rm(args) => commands::agent::rm_by_name(args).await?,
        },
        Command::Run(args) => commands::run::exec(args).await?,
        Command::Status(args) => commands::status::exec(args).await?,
        Command::Sessions(args) => commands::sessions::exec(args).await?,
        Command::Daemon { cmd } => match cmd {
            DaemonCommand::Start(args) => commands::start::exec(args, platform_env).await?,
            DaemonCommand::Stop(args) => commands::stop::exec(args)?,
            DaemonCommand::Status(args) => commands::daemon_status::exec(args)?,
        },
        Command::Stop(args) => commands::stop_agent::exec(args).await?,
        Command::Rm(args) => commands::rm::exec(args).await?,
        Command::Compose { cmd } => match cmd {
            ComposeCommand::Up(args) => commands::compose::up(args).await?,
            ComposeCommand::Down(args) => commands::compose::down(args).await?,
            ComposeCommand::Ps(args) => commands::compose::ps(args).await?,
        },
        Command::Debug(args) => commands::debug::exec(args).await?,
        Command::Cluster { cmd } => match cmd {
            ClusterCommand::Init(args) => commands::cluster::init(args).await?,
            ClusterCommand::AddLearner(args) => commands::cluster::add_learner(args).await?,
            ClusterCommand::ChangeMembership(args) => {
                commands::cluster::change_membership(args).await?
            }
            ClusterCommand::Status(args) => commands::cluster::status(args).await?,
        },
    }

    Ok(())
}
