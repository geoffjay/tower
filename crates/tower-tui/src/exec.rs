//! Effect execution against `/v1` (the only module that does I/O besides
//! the feed and the terminal).

use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tower_client::Client;
use tower_core::{Agent, Event, Machine, Message, Schedule, Task};

use crate::app::Api;
use crate::store::{Fetch, Loaded, TaskDetail};

fn take<T: DeserializeOwned>(v: &mut Value, key: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_value(v[key].take())?)
}

async fn list<T: DeserializeOwned>(c: &Client, path: &str, key: &str) -> anyhow::Result<Vec<T>> {
    take(&mut c.get(path).await?, key)
}

pub async fn fetch(c: &Client, f: &Fetch) -> anyhow::Result<Vec<Loaded>> {
    Ok(match f {
        Fetch::All => {
            let (agents, tasks, schedules, inbox, machines) = tokio::try_join!(
                list::<Agent>(c, "/v1/agents", "agents"),
                list::<Task>(c, "/v1/tasks", "tasks"),
                list::<Schedule>(c, "/v1/schedules", "schedules"),
                list::<Message>(c, "/v1/messages?to=me&status=pending", "messages"),
                list::<Machine>(c, "/v1/machines", "machines"),
            )?;
            vec![
                Loaded::Machines(machines),
                Loaded::Agents(agents),
                Loaded::Tasks(tasks),
                Loaded::Schedules(schedules),
                Loaded::Inbox(inbox),
            ]
        }
        Fetch::Agents => vec![Loaded::Agents(list(c, "/v1/agents", "agents").await?)],
        Fetch::Tasks => vec![Loaded::Tasks(list(c, "/v1/tasks", "tasks").await?)],
        Fetch::TasksSince(since) => vec![Loaded::TasksDelta(
            list(c, &format!("/v1/tasks?since={since}"), "tasks").await?,
        )],
        Fetch::Task(id) => {
            let mut v = c.get(&format!("/v1/tasks/{id}")).await?;
            vec![Loaded::TasksDelta(vec![take(&mut v, "task")?])]
        }
        Fetch::Schedules => vec![Loaded::Schedules(
            list(c, "/v1/schedules", "schedules").await?,
        )],
        Fetch::Inbox => vec![Loaded::Inbox(
            list(c, "/v1/messages?to=me&status=pending", "messages").await?,
        )],
        Fetch::Machines => vec![Loaded::Machines(list(c, "/v1/machines", "machines").await?)],
        Fetch::Output(id) => {
            let mut v = c.get(&format!("/v1/agents/{id}/read?format=ansi")).await?;
            vec![Loaded::Output {
                agent: id.clone(),
                text: take(&mut v, "output")?,
            }]
        }
        Fetch::History(id) => vec![Loaded::History {
            agent: id.clone(),
            messages: list(c, &format!("/v1/messages?agent={id}"), "messages").await?,
        }],
        Fetch::TaskDetail(id) => {
            let mut v = c.get(&format!("/v1/tasks/{id}")).await?;
            vec![Loaded::TaskDetail(Box::new(TaskDetail {
                task: take(&mut v, "task")?,
                trail: take::<Vec<Event>>(&mut v, "trail")?,
                messages: take(&mut v, "messages")?,
            }))]
        }
    })
}

/// Toast shown while a slow call runs (spawn waits for the agent to settle).
pub fn pending_label(a: &Api) -> Option<String> {
    match a {
        Api::Spawn { name, .. } => Some(format!("spawning {name}…")),
        Api::Stop { agent } => Some(format!("stopping {agent}…")),
        Api::Prompt { agent, .. } => Some(format!("prompting {agent}…")),
        _ => None,
    }
}

fn short(id: &str) -> &str {
    &id[id.len().saturating_sub(6)..]
}

/// Run one mutating call; the result is the toast text.
pub async fn api(c: &Client, a: &Api) -> anyhow::Result<String> {
    Ok(match a {
        Api::Prompt { agent, text } => {
            let v = c
                .post(
                    &format!("/v1/agents/{agent}/prompt"),
                    Some(json!({ "text": text })),
                )
                .await?;
            if v["stalled"] == true {
                format!("prompt delivered → {agent} (no state change seen yet)")
            } else {
                format!("prompt delivered → {agent}")
            }
        }
        Api::Interrupt { agent } => {
            c.post(&format!("/v1/agents/{agent}/interrupt"), None)
                .await?;
            format!("interrupted {agent}")
        }
        Api::Respond { msg, approve, text } => {
            let mut body = json!({});
            if let Some(ok) = approve {
                body["approve"] = json!(ok);
            }
            if let Some(t) = text {
                body["parts"] = json!([{ "text": t }]);
            }
            let v = c
                .post(&format!("/v1/messages/{msg}/respond"), Some(body))
                .await?;
            let m = &v["message"];
            let verb = match (m["kind"].as_str(), approve) {
                (Some("approval"), Some(false)) => "denied",
                (Some("approval"), _) => "approved",
                _ => "answered",
            };
            format!(
                "{verb} → delivered to {}",
                m["from_id"].as_str().unwrap_or("?")
            )
        }
        Api::Spawn {
            name,
            kind,
            workdir,
        } => {
            let mut body = json!({ "name": name });
            if let Some(k) = kind {
                body["kind"] = json!(k);
            }
            if let Some(w) = workdir {
                body["workdir"] = json!(w);
            }
            let v = c.post("/v1/agents", Some(body)).await?;
            let a = &v["agent"];
            format!(
                "spawned {} ({}) — {}",
                a["name"].as_str().unwrap_or(name),
                a["kind"].as_str().unwrap_or("?"),
                a["state"].as_str().unwrap_or("launching")
            )
        }
        Api::Stop { agent } => {
            c.post(&format!("/v1/agents/{agent}/stop"), Some(json!({})))
                .await?;
            format!("stopped {agent}")
        }
        Api::Ask { agent, text } => {
            c.post(
                "/v1/messages",
                Some(json!({
                    "to": agent, "to_kind": "agent", "kind": "question",
                    "parts": [{ "text": text }],
                })),
            )
            .await?;
            format!("question sent → {agent}")
        }
        Api::TaskCreate { title } => {
            let v = c.post("/v1/tasks", Some(json!({ "title": title }))).await?;
            format!(
                "queued …{}  {title}",
                short(v["task"]["id"].as_str().unwrap_or(""))
            )
        }
        Api::TaskAssign {
            task,
            agent,
            when_available,
        } => {
            let v = c
                .post(
                    &format!("/v1/tasks/{task}/assign"),
                    Some(json!({ "to": agent, "when_available": when_available })),
                )
                .await?;
            if *when_available {
                format!(
                    "…{} reserved for {agent} (delivered when available)",
                    short(&task.0)
                )
            } else {
                format!(
                    "…{} assigned to {agent} (lease {}s)",
                    short(&task.0),
                    v["task"]["lease_s"].as_i64().unwrap_or(0)
                )
            }
        }
        Api::TaskCancel { task } => {
            c.post(&format!("/v1/tasks/{task}/cancel"), None).await?;
            format!("…{} canceled", short(&task.0))
        }
        Api::SchedulePause { id } => {
            c.post(&format!("/v1/schedules/{id}/pause"), None).await?;
            "schedule paused".into()
        }
        Api::ScheduleResume { id } => {
            c.post(&format!("/v1/schedules/{id}/resume"), None).await?;
            "schedule resumed".into()
        }
        Api::ScheduleRun { id } => {
            let v = c.post(&format!("/v1/schedules/{id}/run"), None).await?;
            match v["fired"]["task_id"].as_str() {
                Some(t) => format!("schedule fired → job …{}", short(t)),
                None => "schedule run requested".into(),
            }
        }
    })
}
