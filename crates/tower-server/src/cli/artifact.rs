//! `tower artifact …` (D§10, phase 7 T5.1): text a job's owner attaches to
//! its open job, and reading it back.

use std::io::Read;
use std::path::PathBuf;

use clap::Subcommand;
use serde_json::json;
use tower_client::Client;

use super::task::print_json;

#[derive(Debug, Subcommand)]
pub enum ArtifactCmd {
    /// Attach text (a file, else stdin; at most 1 MiB) to your open job
    Put {
        task: String,
        name: String,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        media_type: Option<String>,
        #[arg(long = "as", env = "TOWER_AGENT")]
        as_agent: String,
    },
    /// Print an artifact's content
    Get { id: String },
}

pub async fn run(c: &Client, cmd: ArtifactCmd, json_out: bool) -> anyhow::Result<()> {
    match cmd {
        ArtifactCmd::Put {
            task,
            name,
            file,
            media_type,
            as_agent,
        } => {
            let content = match &file {
                Some(p) => std::fs::read_to_string(p)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?,
                None => {
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s
                }
            };
            let v = c
                .post(
                    &format!("/v1/tasks/{task}/artifacts"),
                    Some(json!({
                        "as": as_agent, "name": name, "content": content, "media_type": media_type,
                    })),
                )
                .await?;
            if json_out {
                return print_json(&v);
            }
            let a = &v["artifact"];
            println!(
                "{}  {} ({} bytes) on {task}",
                a["id"].as_str().unwrap_or("?"),
                a["name"].as_str().unwrap_or("?"),
                a["size"]
            );
        }
        ArtifactCmd::Get { id } => {
            let v = c.get(&format!("/v1/artifacts/{id}")).await?;
            if json_out {
                return print_json(&v);
            }
            print!("{}", v["content"].as_str().unwrap_or_default());
        }
    }
    Ok(())
}
