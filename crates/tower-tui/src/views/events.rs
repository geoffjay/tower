//! Events (plan T4.1, D§11): the `/v1/events` feed, filtered by type and
//! subject, following the tail unless you scroll away.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use tower_core::{Event, EventKind};

use super::{Cmd, Ctx};
use crate::app::InputPurpose;
use crate::fmt;
use crate::store::Store;

pub const HINTS: &str = "f type filter · s subject filter · c clear · O output events · G follow";
pub const HELP: &[&str] = &[
    "f       filter by type (substring: `task`, `agent.state`)",
    "s       filter by subject (`agent:<id>`, an agent name, a job id)",
    "c       clear filters",
    "O       show / hide agent.output events (hidden by default)",
    "j k     scroll (pauses follow) · G resume following",
];

#[derive(Debug, Default)]
pub struct Events {
    pub kind_filter: String,
    pub subject_filter: String,
    pub show_output: bool,
    /// Selected event seq when paused; `None` follows the tail.
    pub sel: Option<i64>,
    table: TableState,
}

impl Events {
    pub fn following(&self) -> bool {
        self.sel.is_none()
    }

    /// Filter semantics mirror the server's `?filter=type:` (substring)
    /// and `?subject=` (here: substring of `type:id`, or an agent name).
    pub fn matches(&self, e: &Event, store: &Store) -> bool {
        if !self.show_output && e.kind == EventKind::AgentOutput {
            return false;
        }
        let kf = self.kind_filter.trim();
        let kf = kf.strip_prefix("type:").unwrap_or(kf);
        if !kf.is_empty() && !e.kind.as_str().contains(kf) {
            return false;
        }
        let sf = self.subject_filter.trim();
        if sf.is_empty() {
            return true;
        }
        let (Some(t), Some(id)) = (e.subject_type.as_deref(), e.subject_id.as_deref()) else {
            return false;
        };
        format!("{t}:{id}").contains(sf) || (t == "agent" && store.name_of(&id.into()) == sf)
    }

    pub fn visible<'a>(&self, store: &'a Store) -> Vec<&'a Event> {
        store
            .events
            .iter()
            .filter(|e| self.matches(e, store))
            .collect()
    }

    pub fn key(&mut self, k: KeyEvent, ctx: &Ctx) -> Vec<Cmd> {
        if let Some(n) = super::nav(&k) {
            let seqs: Vec<i64> = self.visible(ctx.store).iter().map(|e| e.seq).collect();
            if n == super::Nav::Bottom {
                self.sel = None;
                return Vec::new();
            }
            // leaving follow mode starts from the newest row
            let from = self.sel.or(seqs.last().copied());
            self.sel = super::step(&seqs, &from, n);
            return Vec::new();
        }
        match k.code {
            KeyCode::Char('f') => vec![Cmd::Input(
                InputPurpose::EventKind,
                self.kind_filter.clone(),
            )],
            KeyCode::Char('s') => vec![Cmd::Input(
                InputPurpose::EventSubject,
                self.subject_filter.clone(),
            )],
            KeyCode::Char('c') => {
                self.kind_filter.clear();
                self.subject_filter.clear();
                Vec::new()
            }
            KeyCode::Char('O') => {
                self.show_output = !self.show_output;
                Vec::new()
            }
            KeyCode::Esc => {
                self.sel = None;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, ctx: &Ctx) {
        let [status, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        let visible = self.visible(ctx.store);
        let mut spans = vec![if self.following() {
            Span::styled("● following", Style::new().fg(Color::Green))
        } else {
            Span::styled("❚❚ paused (G follows)", Style::new().fg(Color::Yellow))
        }];
        spans.push(Span::raw(format!(
            "   {} of {} events",
            visible.len(),
            ctx.store.events.len()
        )));
        if !self.kind_filter.is_empty() {
            spans.push(Span::styled(
                format!("  type~{}", self.kind_filter),
                super::dim(),
            ));
        }
        if !self.subject_filter.is_empty() {
            spans.push(Span::styled(
                format!("  subject~{}", self.subject_filter),
                super::dim(),
            ));
        }
        if self.show_output {
            spans.push(Span::styled("  +output", super::dim()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), status);

        if visible.is_empty() {
            let msg = if ctx.store.events.is_empty() {
                "no events yet — live events appear here as they happen"
            } else {
                "no events match — `c` clears the filters"
            };
            f.render_widget(Paragraph::new(Span::styled(msg, super::dim())), body);
            return;
        }
        let seqs: Vec<i64> = visible.iter().map(|e| e.seq).collect();
        let idx = match self.sel {
            None => seqs.len() - 1,
            Some(s) => seqs.iter().position(|x| *x == s).unwrap_or(seqs.len() - 1),
        };
        self.table.select(Some(idx));
        let rows: Vec<Row> = visible.iter().map(|e| row(e, ctx)).collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Length(17),
                Constraint::Length(16),
                Constraint::Min(10),
            ],
        )
        .header(Row::new(["TIME", "TYPE", "SUBJECT", "DETAIL"]).style(super::header()))
        .row_highlight_style(if self.following() {
            Style::new()
        } else {
            super::selected()
        });
        f.render_stateful_widget(table, body, &mut self.table);
    }
}

fn kind_style(k: EventKind) -> Style {
    let s = k.as_str();
    let c = if s.starts_with("agent.") {
        Color::Green
    } else if s.starts_with("task.") {
        Color::Blue
    } else if s.starts_with("message.") || s.starts_with("approval.") {
        Color::Yellow
    } else if s.starts_with("schedule.") {
        Color::Magenta
    } else {
        Color::Gray
    };
    Style::new().fg(c)
}

/// Subject column: agent names, short job ids, schedule titles.
pub fn subject(e: &Event, store: &Store) -> String {
    let (Some(t), Some(id)) = (e.subject_type.as_deref(), e.subject_id.as_deref()) else {
        return "—".into();
    };
    let tail = |s: &str| -> String {
        let n = s.chars().count();
        s.chars().skip(n.saturating_sub(6)).collect()
    };
    match t {
        "agent" => store.name_of(&id.into()).to_string(),
        "task" => format!("job …{}", tail(id)),
        "schedule" => store
            .schedules
            .iter()
            .find(|s| s.id.0 == id)
            .map_or_else(|| format!("sched …{}", tail(id)), |s| s.title.clone()),
        other => format!("{other} …{}", tail(id)),
    }
}

/// Detail column: the payload in operator words.
pub fn detail(e: &Event, store: &Store) -> String {
    let p = &e.payload;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or("?").to_string();
    match e.kind {
        EventKind::AgentStateChange => format!("{} → {}", s(&p["from"]), s(&p["to"])),
        EventKind::AgentCreated => {
            let mut d = format!("{} ({})", s(&p["name"]), s(&p["kind"]));
            if p["adopted"] == true {
                d.push_str(" adopted");
            }
            d
        }
        EventKind::AgentRemoved => s(&p["name"]),
        EventKind::AgentOutput => format!("{} bytes", p["bytes"].as_i64().unwrap_or(0)),
        k if k.as_str().starts_with("task.") => {
            let title = e
                .subject_id
                .as_ref()
                .and_then(|id| store.tasks.get(&id.clone().into()))
                .map(|t| format!("  {}", fmt::trunc(&t.title, 30)))
                .unwrap_or_default();
            format!("{}{title}", super::tasks::trail_line(e, store))
        }
        EventKind::MessageCreated if p["text"].is_string() => {
            format!("{}: {}", s(&p["kind"]), fmt::trunc(&s(&p["text"]), 60))
        }
        EventKind::MessageCreated => format!(
            "{} {} → {}",
            s(&p["kind"]),
            s(&p["from"]["id"]),
            s(&p["to"]["id"])
        ),
        EventKind::MessageStatusChange => s(&p["status"]),
        EventKind::ScheduleFired => format!("fired → job …{}", {
            let id = s(&p["task_id"]);
            let n = id.chars().count();
            id.chars().skip(n.saturating_sub(6)).collect::<String>()
        }),
        EventKind::ScheduleCreated => format!(
            "{} · {} {}",
            s(&p["title"]),
            s(&p["cron"]),
            s(&p["timezone"])
        ),
        EventKind::ScheduleSkipped | EventKind::SchedulePaused => {
            p["reason"].as_str().unwrap_or("").to_string()
        }
        _ => fmt::trunc(&p.to_string(), 80),
    }
}

fn row<'a>(e: &'a Event, ctx: &Ctx) -> Row<'a> {
    Row::new(vec![
        Cell::from(Span::styled(ctx.tz.clock(e.ts, false), super::dim())),
        Cell::from(Span::styled(e.kind.as_str(), kind_style(e.kind))),
        Cell::from(subject(e, ctx.store)),
        Cell::from(detail(e, ctx.store)),
    ])
}
