use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "tower",
    version,
    about = "control and visibility for agent herds"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server
    Serve,
    /// Run a remote-machine node agent (phase 5)
    Node,
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async_main(cli))
}

async fn async_main(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Serve => tower_server::serve::serve().await,
        Command::Node => anyhow::bail!("node agent arrives in phase 5"),
    }
}
