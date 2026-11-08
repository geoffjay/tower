//! Interactive questions for `tower task create` and `tower workflow run`
//! (D§10 "Asked, not remembered"): run without the positional argument in a
//! terminal and the CLI asks for every value no flag gave, each with the
//! control that fits it, then prints the equivalent one-line command and
//! asks to go ahead. Prompts draw on stderr, so `--json` stays clean.

use std::collections::BTreeSet;
use std::fmt;
use std::io::IsTerminal;

use inquire::validator::Validation;
use inquire::{Confirm, CustomType, DateSelect, Editor, InquireError, MultiSelect, Select, Text};
use serde_json::Value;
use tower_client::Client;

/// Both ends a person sits at: answers come from stdin, questions go to stderr.
fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Missing positional argument: fine in a terminal (we ask), an error elsewhere.
pub fn require_terminal(what: &str) -> anyhow::Result<()> {
    if interactive() {
        Ok(())
    } else {
        anyhow::bail!("missing {what}: pass it, or run in a terminal to be asked")
    }
}

/// Esc / ctrl-c end the whole command, nothing created.
fn answer<T>(r: Result<T, InquireError>) -> anyhow::Result<T> {
    r.map_err(|e| match e {
        InquireError::OperationCanceled | InquireError::OperationInterrupted => {
            anyhow::anyhow!("canceled")
        }
        e => anyhow::anyhow!(e),
    })
}

fn required(s: &str) -> Result<Validation, inquire::CustomUserError> {
    Ok(if s.trim().is_empty() {
        Validation::Invalid("required".into())
    } else {
        Validation::Valid
    })
}

/// Show the command, then ask to go ahead.
fn go_ahead(command: &str, question: &str) -> anyhow::Result<bool> {
    eprintln!("\n  {command}\n");
    answer(Confirm::new(question).with_default(true).prompt())
}

/// One shell word, single-quoted only when it needs to be.
fn shell_word(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Every value `tower task create` takes; flags fill it before any question.
#[derive(Debug, Default)]
pub struct NewTask {
    pub title: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub priority: Option<i64>,
    pub assign: Option<String>,
    pub when_available: bool,
    pub at: Option<String>,
    pub lease_s: Option<i64>,
    pub pre_run: Option<String>,
    pub post_run: Option<String>,
}

impl NewTask {
    /// The `tower task create ...` line that does the same thing.
    fn command_line(&self) -> String {
        let mut w = vec!["tower task create".to_string()];
        w.push(shell_word(self.title.as_deref().unwrap_or("")));
        let mut flag = |name: &str, v: &str| w.push(format!("--{name} {}", shell_word(v)));
        if let Some(d) = &self.description {
            flag("description", d);
        }
        for t in &self.tags {
            flag("tag", t);
        }
        if let Some(p) = self.priority {
            flag("priority", &p.to_string());
        }
        if let Some(a) = &self.assign {
            flag("assign", a);
        }
        if let Some(at) = &self.at {
            flag("at", at);
        }
        if let Some(l) = self.lease_s {
            flag("lease-s", &l.to_string());
        }
        if let Some(p) = &self.pre_run {
            flag("pre-run", p);
        }
        if let Some(p) = &self.post_run {
            flag("post-run", p);
        }
        if self.when_available {
            w.push("--when-available".into());
        }
        w.join(" ")
    }
}

/// A row of the assignee list.
struct Assignee {
    name: Option<String>,
    label: String,
    busy: bool,
}

impl fmt::Display for Assignee {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

const OPEN: [&str; 3] = ["assigned", "working", "input-required"];

/// Ask for what `t` is missing. Ok(false): the person said no at the end.
pub async fn task_create(c: &Client, t: &mut NewTask) -> anyhow::Result<bool> {
    let (agents, tasks, schedules) = tokio::try_join!(
        c.get("/v1/agents"),
        c.get("/v1/tasks"),
        c.get("/v1/schedules")
    )?;
    let tasks = tasks["tasks"].as_array().cloned().unwrap_or_default();

    if t.title.is_none() {
        let title = answer(Text::new("Title:").with_validator(required).prompt())?;
        t.title = Some(title.trim().to_string());
    }
    if t.description.is_none() {
        let d = answer(
            Editor::new("Job prompt:")
                .with_help_message("what the agent should do; enter alone leaves it empty")
                .with_file_extension(".md")
                .prompt(),
        )?;
        let d = d.trim();
        t.description = (!d.is_empty()).then(|| d.to_string());
    }

    let owners: BTreeSet<&str> = tasks
        .iter()
        .filter(|j| OPEN.contains(&j["state"].as_str().unwrap_or("")))
        .filter_map(|j| j["owner_id"].as_str())
        .collect();
    let mut rows = vec![Assignee {
        name: None,
        label: "nobody: leave it queued".into(),
        busy: false,
    }];
    for a in agents["agents"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let (Some(name), Some(id)) = (a["name"].as_str(), a["id"].as_str()) else {
            continue;
        };
        let state = a["state"].as_str().unwrap_or("unknown");
        if state == "dead" {
            continue;
        }
        let has_job = owners.contains(id);
        let kind = a["kind"].as_str().unwrap_or("?");
        let job = if has_job { ", has a job" } else { "" };
        rows.push(Assignee {
            name: Some(name.to_string()),
            label: format!("{name}  ({kind}, {state}{job})"),
            busy: has_job || state != "idle",
        });
    }
    // an assignee from --assign is looked up so the delivery default fits it
    let mut busy_agent = rows
        .iter()
        .any(|r| r.busy && r.name.is_some() && r.name == t.assign);
    if t.assign.is_none() {
        let pick = answer(
            Select::new("Assign to:", rows)
                .with_help_message("type to filter")
                .with_page_size(12)
                .prompt(),
        )?;
        busy_agent = pick.busy;
        t.assign = pick.name;
    }

    if t.assign.is_some() && !t.when_available && t.at.is_none() {
        const NOW: &str = "now";
        const FREE: &str = "when it is free (reserve)";
        const AT: &str = "not before a time (reserve)";
        let when = answer(
            Select::new("Deliver:", vec![NOW, FREE, AT])
                .with_starting_cursor(if busy_agent { 1 } else { 0 })
                .with_help_message("an agent holds one job at a time; a busy one refuses `now`")
                .prompt(),
        )?;
        if when == FREE {
            t.when_available = true;
        } else if when == AT {
            let today = chrono::Local::now().date_naive();
            let day = answer(
                DateSelect::new("Day:")
                    .with_min_date(today)
                    .with_starting_date(today)
                    .prompt(),
            )?;
            let time = answer(
                Text::new("Time (HH:MM, local):")
                    .with_validator(|s: &str| {
                        Ok(match chrono::NaiveTime::parse_from_str(s.trim(), "%H:%M") {
                            Ok(_) => Validation::Valid,
                            Err(_) => Validation::Invalid("use HH:MM, 24-hour".into()),
                        })
                    })
                    .prompt(),
            )?;
            t.at = Some(format!("{} {}", day.format("%Y-%m-%d"), time.trim()));
        }
    }

    if t.tags.is_empty() {
        let mut known = BTreeSet::new();
        let sch = schedules["schedules"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for v in tasks.iter().chain(&sch) {
            for tag in v["tags"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                if let Some(s) = tag.as_str() {
                    known.insert(s.to_string());
                }
            }
        }
        let mut tags = Vec::new();
        if !known.is_empty() {
            tags = answer(
                MultiSelect::new("Tags:", known.into_iter().collect())
                    .with_help_message("space picks, enter accepts (none is fine); new tags next")
                    .prompt(),
            )?;
        }
        let more = answer(
            Text::new(if tags.is_empty() {
                "Tags:"
            } else {
                "New tags:"
            })
            .with_help_message("comma-separated; enter for none")
            .prompt(),
        )?;
        for tag in more.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !tags.iter().any(|x| x == tag) {
                tags.push(tag.to_string());
            }
        }
        t.tags = tags;
    }

    if t.priority.is_none() {
        let p = answer(
            CustomType::<i64>::new("Priority:")
                .with_default(0)
                .with_help_message("higher is dispatched first")
                .with_error_message("a whole number")
                .prompt(),
        )?;
        t.priority = (p != 0).then_some(p);
    }

    if t.lease_s.is_none()
        && t.pre_run.is_none()
        && t.post_run.is_none()
        && answer(
            Confirm::new("Set a lease or pre-run / post-run prompts?")
                .with_default(false)
                .prompt(),
        )?
    {
        let lease = answer(
            CustomType::<i64>::new("Lease (seconds):")
                .with_default(60)
                .with_help_message("how long the owner may go without a heartbeat")
                .with_error_message("a whole number of seconds")
                .prompt(),
        )?;
        t.lease_s = (lease != 60).then_some(lease);
        let lifecycle = |q: &str, help: &str| -> anyhow::Result<Option<String>> {
            let s = answer(Text::new(q).with_help_message(help).prompt())?;
            Ok((!s.trim().is_empty()).then(|| s.trim().to_string()))
        };
        t.pre_run = lifecycle(
            "Pre-run prompt:",
            "typed into the harness before the job, e.g. /clear; enter for none",
        )?;
        t.post_run = lifecycle(
            "Post-run prompt:",
            "typed after the owner reports completed or failed; enter for none",
        )?;
    }

    go_ahead(&t.command_line(), "Create this job?")
}

/// A row of the workflow list.
struct WorkflowRow {
    name: String,
    label: String,
}

impl fmt::Display for WorkflowRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// The `tower workflow run ...` line that does the same thing.
fn run_command_line(name: &str, inputs: &[String], wait: bool) -> String {
    let mut w = vec!["tower workflow run".to_string(), shell_word(name)];
    for kv in inputs {
        w.push(format!("--input {}", shell_word(kv)));
    }
    if wait {
        w.push("--wait".into());
    }
    w.join(" ")
}

/// Ask for the workflow, its declared inputs, and whether to follow the run.
/// Ok(false): the person said no at the end.
pub async fn workflow_run(
    c: &Client,
    name: &mut Option<String>,
    inputs: &mut Vec<String>,
    wait: &mut bool,
) -> anyhow::Result<bool> {
    let chosen = match name.take() {
        Some(n) => n,
        None => {
            let v = c.get("/v1/workflows").await?;
            let rows: Vec<WorkflowRow> = v["workflows"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter(|w| w["enabled"] == true)
                .filter_map(|w| {
                    let name = w["name"].as_str()?.to_string();
                    let about: String = w["description"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(70)
                        .collect();
                    let label = if about.is_empty() {
                        name.clone()
                    } else {
                        format!("{name}  {about}")
                    };
                    Some(WorkflowRow { name, label })
                })
                .collect();
            if rows.is_empty() {
                anyhow::bail!("no enabled workflows (see `tower workflow list`)");
            }
            answer(
                Select::new("Workflow:", rows)
                    .with_help_message("type to filter; disabled workflows are not listed")
                    .prompt(),
            )?
            .name
        }
    };

    let show = c.get(&format!("/v1/workflows/{chosen}")).await?;
    let given: BTreeSet<String> = inputs
        .iter()
        .filter_map(|kv| kv.split_once('=').map(|(k, _)| k.trim().to_string()))
        .collect();
    let empty = serde_json::Map::new();
    let declared = show["definition"]["inputs"].as_object().unwrap_or(&empty);
    for (key, spec) in declared.iter().filter(|(k, _)| !given.contains(*k)) {
        if let Some(v) = ask_input(key, spec)? {
            inputs.push(format!("{key}={v}"));
        }
    }

    if !*wait {
        *wait = answer(
            Confirm::new("Follow the run until it ends?")
                .with_default(true)
                .prompt(),
        )?;
    }
    let ok = go_ahead(&run_command_line(&chosen, inputs, *wait), "Start this run?")?;
    *name = Some(chosen);
    Ok(ok)
}

/// One declared input, with the control its default's type calls for.
/// None: optional and left empty (the server applies its default, if any).
fn ask_input(key: &str, spec: &Value) -> anyhow::Result<Option<String>> {
    let req = spec["required"] == true;
    let q = format!("{key}{}:", if req { " (required)" } else { "" });
    // inquire draws an empty help message as `[]`, so an undescribed input
    // gets a hint for its control instead
    let described = spec["description"].as_str().filter(|s| !s.is_empty());
    let help = |fallback| described.unwrap_or(fallback);
    let v = match &spec["default"] {
        Value::Bool(d) => answer(
            Confirm::new(&q)
                .with_default(*d)
                .with_help_message(help("yes or no"))
                .prompt(),
        )?
        .to_string(),
        Value::Number(n) if n.is_i64() => answer(
            CustomType::<i64>::new(&q)
                .with_default(n.as_i64().unwrap_or(0))
                .with_help_message(help("a whole number"))
                .with_error_message("a whole number")
                .prompt(),
        )?
        .to_string(),
        Value::Number(n) => answer(
            CustomType::<f64>::new(&q)
                .with_default(n.as_f64().unwrap_or(0.0))
                .with_help_message(help("a number"))
                .with_error_message("a number")
                .prompt(),
        )?
        .to_string(),
        Value::Null => {
            let mut p = Text::new(&q).with_help_message(help(if req {
                "required"
            } else {
                "optional; enter to skip"
            }));
            if req {
                p = p.with_validator(required);
            }
            let s = answer(p.prompt())?;
            if s.trim().is_empty() {
                return Ok(None);
            }
            s.trim().to_string()
        }
        other => {
            let d = other
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| other.to_string());
            answer(
                Text::new(&q)
                    .with_default(&d)
                    .with_help_message(help("enter keeps the default"))
                    .prompt(),
            )?
        }
    };
    Ok(Some(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_words_quote_only_what_the_shell_would_split_or_expand() {
        assert_eq!(shell_word("kb-reviewer"), "kb-reviewer");
        assert_eq!(shell_word("stack:kb"), "stack:kb");
        assert_eq!(shell_word("2026-11-09 22:00"), "'2026-11-09 22:00'");
        assert_eq!(shell_word("/clear"), "/clear");
        assert_eq!(shell_word("it's $HOME"), r"'it'\''s $HOME'");
        assert_eq!(shell_word(""), "''");
    }

    #[test]
    fn the_echoed_command_reproduces_every_answer() {
        let t = NewTask {
            title: Some("review PR #45".into()),
            tags: vec!["stack:kb".into()],
            priority: Some(2),
            assign: Some("kb-reviewer".into()),
            when_available: true,
            pre_run: Some("/clear".into()),
            ..Default::default()
        };
        assert_eq!(
            t.command_line(),
            "tower task create 'review PR #45' --tag stack:kb --priority 2 \
             --assign kb-reviewer --pre-run /clear --when-available"
        );
        assert_eq!(
            run_command_line("kb-daily", &["pr=45".into(), "note=a b".into()], true),
            "tower workflow run kb-daily --input pr=45 --input 'note=a b' --wait"
        );
    }
}
