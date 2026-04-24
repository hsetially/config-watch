use clap::Parser;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "APP_CONFIG", default_value = "deploy/dev/agent.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let args = Args::parse();
    let cfg = config_agent::config::AgentConfig::from_file(&args.config)?;
    config_agent::app::run(cfg).await
}
