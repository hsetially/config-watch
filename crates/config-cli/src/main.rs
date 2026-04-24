use clap::{Parser, Subcommand};

mod hosts;
mod stat;
mod tail;

#[derive(Debug, Parser)]
#[command(name = "config-cli", about = "Config Watch CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List registered hosts
    Hosts {
        /// Filter by status (healthy, degraded, offline)
        #[arg(short, long)]
        status: Option<String>,
    },
    /// Tail live change events
    Tail {
        /// Filter by environment
        #[arg(short, long)]
        env: Option<String>,
        /// Filter by host ID
        #[arg(long)]
        host_id: Option<String>,
        /// Filter by path prefix
        #[arg(long)]
        path_prefix: Option<String>,
        /// Show diff content after each event
        #[arg(long)]
        diff: bool,
    },
    /// Get file metadata
    Stat {
        /// Host ID
        host_id: String,
        /// File path
        path: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let cli = Cli::parse();

    let base_url =
        std::env::var("CONFIG_WATCH_URL").unwrap_or_else(|_| "http://localhost:8082".to_string());

    match cli.command {
        Command::Hosts { status } => hosts::list_hosts(&base_url, status.as_deref()).await?,
        Command::Tail {
            env,
            host_id,
            path_prefix,
            diff,
        } => tail::tail_changes(&base_url, env, host_id, path_prefix, diff).await?,
        Command::Stat { host_id, path } => stat::file_stat(&base_url, &host_id, &path).await?,
    }

    Ok(())
}
