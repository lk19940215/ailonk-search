mod cache;
mod cli;
mod commands;
mod server;
mod browser;
mod search;
mod extract;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ailonk_search=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = cli::Cli::parse();
    tracing::info!("Starting ailonk-search");

    commands::run(cli).await
}
