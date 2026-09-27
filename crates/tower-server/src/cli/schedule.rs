//! `tower schedule …` (D§10, plan 2b T2.2): recurring jobs.

use std::collections::HashMap;

use clap::Subcommand;
use serde_json::{json, Value};
use tower_client::Client;

use super::task::{agent_names, clock, name, print_json};

#[derive(Debug, Subcommand)]
pub enum ScheduleCmd {
    /// Create a recurring job (fires on a cadence, creates an ordinary job)
    Create {
        title: String,
        /// Every day at HH:MM (local to --tz)
        #[arg(long, conflicts_with = "cron")]
        daily: Option<String>,
        /// Cron expression, 5 or 6 fields (e.g. '0 9 * * MON-FRI')
        #[arg(long)]
        cron: Option<String>,
        /// IANA timezone (default: this machine's)
        #[arg(long)]
        tz: Option<String>,
        /// Reserve each job for this agent (delivered when it's free);
        /// without it, jobs go to the general queue for you to assign
        #[arg(long)]
        assign: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        priority: Option<i64>,
        /// Lease window in seconds for each job (default 60)
        #[arg(long)]
        lease_s: Option<i64>,
    },
    /// Schedules with next and last run
    List,
    /// A schedule and its recent jobs
    Show { id: String },
    /// Stop firing
    Pause { id: String },
    /// Resume firing from now (missed runs while paused are not caught up)
    Resume { id: String },
    /// Fire one extra run now (cadence unchanged)
    Run { id: String },
    /// Delete the schedule (jobs it created are left alone)
    Rm { id: String },
}

pub async fn run(c: &Client, cmd: ScheduleCmd, json_out: bool) -> anyhow::Result<()> {
    match cmd {
        ScheduleCmd::Create {
            title,
            daily,
            cron,
            tz,
            assign,
            description,
            tags,
            priority,
            lease_s,
        } => {
            if daily.is_none() && cron.is_none() {
                anyhow::bail!("give --daily HH:MM or --cron '<expr>'");
            }
            let body = json!({
                "title": title, "daily": daily, "cron": cron, "timezone": tz,
                "target": assign, "description": description, "tags": tags,
                "priority": priority, "lease_s": lease_s,
            });
            let v = c.post("/v1/schedules", Some(body)).await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            println!("{}", summary(&v["schedule"], &names));
        }
        ScheduleCmd::List => {
            let v = c.get("/v1/schedules").await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            print_list(
                v["schedules"].as_array().map(Vec::as_slice).unwrap_or(&[]),
                &names,
            );
        }
        ScheduleCmd::Show { id } => {
            let v = c.get(&format!("/v1/schedules/{id}")).await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            print_show(&v, &names);
        }
        ScheduleCmd::Pause { id } => {
            let v = c.post(&format!("/v1/schedules/{id}/pause"), None).await?;
            if json_out {
                return print_json(&v);
            }
            println!("{id}  paused");
        }
        ScheduleCmd::Resume { id } => {
            let v = c.post(&format!("/v1/schedules/{id}/resume"), None).await?;
            if json_out {
                return print_json(&v);
            }
            let next = v["schedule"]["next_run_at"]
                .as_i64()
                .map(|n| clock(n, true));
            println!(
                "{id}  resumed; next: {}",
                next.unwrap_or_else(|| "—".into())
            );
        }
        ScheduleCmd::Run { id } => {
            let v = c.post(&format!("/v1/schedules/{id}/run"), None).await?;
            if json_out {
                return print_json(&v);
            }
            let f = &v["fired"];
            match f["outcome"].as_str() {
                Some("created") => {
                    let mut s = format!("{id}  ran → job {}", f["task_id"].as_str().unwrap_or("?"));
                    if let Some(old) = f["expired"].as_str() {
                        s.push_str(&format!(" (replaced undelivered {old})"));
                    }
                    println!("{s}");
                }
                Some("skipped") => println!(
                    "{id}  skipped: previous run {} is still being worked",
                    f["running"].as_str().unwrap_or("?")
                ),
                _ => println!("{id}  no new job (already exists for this time)"),
            }
        }
        ScheduleCmd::Rm { id } => {
            let v = c.delete(&format!("/v1/schedules/{id}")).await?;
            if json_out {
                return print_json(&v);
            }
            println!("{id}  removed");
        }
    }
    Ok(())
}

/// `daily HH:MM` for `M H * * *`, else the cron expression.
fn cadence(cron: &str) -> String {
    let f: Vec<&str> = cron.split_whitespace().collect();
    match f.as_slice() {
        [m, h, "*", "*", "*"] => match (h.parse::<u32>(), m.parse::<u32>()) {
            (Ok(h), Ok(m)) => format!("daily {h:02}:{m:02}"),
            _ => cron.to_string(),
        },
        _ => cron.to_string(),
    }
}

fn target(s: &Value, names: &HashMap<String, String>) -> String {
    if s["target_agent_id"].is_string() {
        name(names, &s["target_agent_id"]).to_string()
    } else {
        "(queue)".into()
    }
}

fn when(v: &Value) -> String {
    v.as_i64()
        .map(|t| clock(t, true))
        .unwrap_or_else(|| "—".into())
}

/// `s_…  daily 09:00 America/Los_Angeles → backend  next: …`
fn summary(s: &Value, names: &HashMap<String, String>) -> String {
    format!(
        "{}  {} {} → {}  next: {}",
        s["id"].as_str().unwrap_or("?"),
        cadence(s["cron"].as_str().unwrap_or("")),
        s["timezone"].as_str().unwrap_or(""),
        target(s, names),
        when(&s["next_run_at"]),
    )
}

fn print_list(schedules: &[Value], names: &HashMap<String, String>) {
    println!(
        "{:<28} {:<16} {:<20} {:<10} {:<20} {:<20} TITLE",
        "ID", "CADENCE", "ZONE", "TARGET", "NEXT", "LAST"
    );
    for s in schedules {
        let next = if s["enabled"] == true {
            when(&s["next_run_at"])
        } else {
            "paused".into()
        };
        println!(
            "{:<28} {:<16} {:<20} {:<10} {:<20} {:<20} {}",
            s["id"].as_str().unwrap_or("?"),
            cadence(s["cron"].as_str().unwrap_or("")),
            s["timezone"].as_str().unwrap_or(""),
            target(s, names),
            next,
            when(&s["last_run_at"]),
            s["title"].as_str().unwrap_or(""),
        );
    }
}

fn print_show(v: &Value, names: &HashMap<String, String>) {
    let s = &v["schedule"];
    println!(
        "schedule {}  {}",
        s["id"].as_str().unwrap_or("?"),
        s["title"].as_str().unwrap_or("")
    );
    println!(
        "  cadence  {} ({}) {}",
        cadence(s["cron"].as_str().unwrap_or("")),
        s["cron"].as_str().unwrap_or(""),
        s["timezone"].as_str().unwrap_or("")
    );
    println!("  target   {}", target(s, names));
    println!(
        "  state    {}",
        if s["enabled"] == true {
            "active"
        } else {
            "paused"
        }
    );
    println!("  next     {}", when(&s["next_run_at"]));
    println!("  last     {}", when(&s["last_run_at"]));
    println!("  jobs (newest first)");
    for j in v["jobs"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        println!(
            "    {}  {:<15} {}",
            when(&j["occurrence_at"]),
            j["state"].as_str().unwrap_or("?"),
            j["id"].as_str().unwrap_or("?"),
        );
    }
}
