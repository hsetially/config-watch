use clap::Parser;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "APP_CONFIG", default_value = "deploy/dev/control-plane.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    let cfg = config_control_plane::config::ControlPlaneConfig::from_file(&args.config)?;
    config_control_plane::app::run(cfg).await
}
