mod cli;
mod config;
mod paths;
mod storage;

pub use config::Config;
pub use paths::Paths;
pub use storage::{open as open_db, EventLog};

fn main() -> anyhow::Result<()> {
    cli::run()
}
