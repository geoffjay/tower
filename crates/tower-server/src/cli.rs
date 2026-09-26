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
    match cli.command {
        Command::Serve => {
            println!("tower serve: server shell lands in milestone 3");
            Ok(())
        }
        Command::Node => {
            anyhow::bail!("node agent arrives in phase 5")
        }
    }
}
