use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "config-agent")]
struct Args {
    #[arg(long, env = "APP_CONFIG", default_value = "deploy/dev/agent.toml")]
    config: String,

    /// Run diagnostic checks and exit (do not start the agent).
    #[arg(long)]
    diagnose: bool,

    /// Output format for diagnostics: text or json (default: text).
    #[arg(long, default_value = "text")]
    format: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    let cfg = config_agent::config::AgentConfig::from_file(&args.config)?;

    if args.diagnose {
        config_agent::diagnose::run(&cfg, &args.format)
    } else {
        config_agent::app::run(cfg).await
    }
}
