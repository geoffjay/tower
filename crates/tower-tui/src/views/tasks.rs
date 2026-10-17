//! Tasks (plan T3.2, D§11): owned jobs with live lease countdowns, the
//! queue (with reserved-for targets), recently closed jobs, schedules with
//! next run; detail shows the assignment trail and messages.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;
use tower_core::{Event, Schedule, ScheduleId, Task, TaskId, TaskState};

use super::{Cmd, Ctx};
use crate::app::{Api, Effect, InputPurpose};
use crate::fmt;
use crate::store::{Fetch, Store};

pub const HELP: &[&str] = &[
    "h l     previous / next pane (owned, queue, recent, schedules)",
    "enter   job detail: trail + messages (esc closes)",
    "n       queue a new job",
    "a       assign the selected job to an agent",
    "x x     cancel the selected job (press twice)",
    "p       pause / resume the selected schedule",
    "R       run the selected schedule now",
];

const RECENT: usize = 10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    #[default]
    Owned,
    Queue,
    Recent,
    Schedules,
}

const PANES: [Pane; 4] = [Pane::Owned, Pane::Queue, Pane::Recent, Pane::Schedules];

#[derive(Debug, Default)]
pub struct Tasks {
    pub pane: Pane,
    pub sel: [Option<TaskId>; 3],
    pub sel_schedule: Option<ScheduleId>,
    /// Job whose detail is open.
    pub detail: Option<TaskId>,
    /// Schedule whose detail is open (exclusive with `detail`).
    pub schedule_detail: Option<ScheduleId>,
    /// Lines scrolled up from the bottom of the open detail (0 = bottom).
    detail_scroll: usize,
    confirm_cancel: Option<TaskId>,
    tables: [TableState; 4],
}

impl Tasks {
    /// The fetch the open side panel needs, if any.
    pub fn open_detail(&self) -> Vec<Fetch> {
        if let Some(id) = &self.detail {
            vec![Fetch::TaskDetail(id.clone())]
        } else if let Some(id) = &self.schedule_detail {
            vec![Fetch::ScheduleDetail(id.clone())]
        } else {
            Vec::new()
        }
    }

    /// True when either detail panel is open.
    fn any_detail(&self) -> bool {
        self.detail.is_some() || self.schedule_detail.is_some()
    }

    fn close_detail(&mut self) {
        self.detail = None;
        self.schedule_detail = None;
        self.detail_scroll = 0;
    }
}

/// Assigned / working / input-required, soonest lease first.
pub fn owned(store: &Store) -> Vec<&Task> {
    let mut v: Vec<&Task> = store
        .tasks
        .values()
        .filter(|t| {
            matches!(
                t.state,
                TaskState::Assigned | TaskState::Working | TaskState::InputRequired
            )
        })
        .collect();
    v.sort_by_key(|t| (t.lease_expires_at.unwrap_or(i64::MAX), t.created_at));
    v
}

/// Queued, dispatch order (priority desc, then oldest).
pub fn queue(store: &Store) -> Vec<&Task> {
    let mut v: Vec<&Task> = store
        .tasks
        .values()
        .filter(|t| t.state == TaskState::Queued)
        .collect();
    v.sort_by_key(|t| (std::cmp::Reverse(t.priority), t.created_at));
    v
}

/// Latest closed jobs.
pub fn recent(store: &Store) -> Vec<&Task> {
    let mut v: Vec<&Task> = store
        .tasks
        .values()
        .filter(|t| t.state.is_terminal())
        .collect();
    v.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
    v.truncate(RECENT);
    v
}

fn short(id: &TaskId) -> String {
    let n = id.0.chars().count();
    id.0.chars().skip(n.saturating_sub(6)).collect()
}

impl Tasks {
    pub fn hints(&self) -> &'static str {
        if self.any_detail() {
            "esc close · j/k scroll · a assign · x x cancel"
        } else if self.pane == Pane::Schedules {
            "h/l pane · enter detail · p pause/resume · R run now · n new job"
        } else {
            "h/l pane · enter detail · n new job · a assign · x x cancel"
        }
    }

    fn pane_tasks<'a>(&self, store: &'a Store, pane: Pane) -> Vec<&'a Task> {
        match pane {
            Pane::Owned => owned(store),
            Pane::Queue => queue(store),
            Pane::Recent => recent(store),
            Pane::Schedules => Vec::new(),
        }
    }

    fn pane_idx(pane: Pane) -> usize {
        PANES.iter().position(|p| *p == pane).unwrap_or(0)
    }

    /// The job the keys act on: open detail, else the pane's selection.
    fn target(&self, store: &Store) -> Option<TaskId> {
        if let Some(d) = &self.detail {
            return Some(d.clone());
        }
        if self.pane == Pane::Schedules {
            return None;
        }
        let ids: Vec<TaskId> = self
            .pane_tasks(store, self.pane)
            .iter()
            .map(|t| t.id.clone())
            .collect();
        super::current(&ids, &self.sel[Self::pane_idx(self.pane)])
    }

    fn target_schedule<'a>(&self, store: &'a Store) -> Option<&'a Schedule> {
        let ids: Vec<&ScheduleId> = store.schedules.iter().map(|s| &s.id).collect();
        super::index_of(&ids, &self.sel_schedule.as_ref()).map(|i| &store.schedules[i])
    }

    pub fn key(&mut self, k: KeyEvent, ctx: &Ctx) -> Vec<Cmd> {
        let store = ctx.store;
        let confirming = self.confirm_cancel.take();
        if self.any_detail() {
            if k.code == KeyCode::Esc {
                self.close_detail();
                return Vec::new();
            }
            // scroll the open detail; g/G snap, ctrl-d/u page
            if let Some(n) = super::nav(&k) {
                self.detail_scroll = match n {
                    super::Nav::Up => self.detail_scroll + 1,
                    super::Nav::Down => self.detail_scroll.saturating_sub(1),
                    super::Nav::PageUp => self.detail_scroll + 10,
                    super::Nav::PageDown => self.detail_scroll.saturating_sub(10),
                    super::Nav::Top => usize::MAX / 2,
                    super::Nav::Bottom => 0,
                };
                return Vec::new();
            }
        } else {
            if let Some(n) = super::nav(&k) {
                if self.pane == Pane::Schedules {
                    let ids: Vec<ScheduleId> =
                        store.schedules.iter().map(|s| s.id.clone()).collect();
                    self.sel_schedule = super::step(&ids, &self.sel_schedule, n);
                } else {
                    let i = Self::pane_idx(self.pane);
                    let ids: Vec<TaskId> = self
                        .pane_tasks(store, self.pane)
                        .iter()
                        .map(|t| t.id.clone())
                        .collect();
                    self.sel[i] = super::step(&ids, &self.sel[i], n);
                }
                return Vec::new();
            }
            match k.code {
                KeyCode::Char('h') | KeyCode::Left => {
                    let i = Self::pane_idx(self.pane);
                    self.pane = PANES[(i + PANES.len() - 1) % PANES.len()];
                    return Vec::new();
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    let i = Self::pane_idx(self.pane);
                    self.pane = PANES[(i + 1) % PANES.len()];
                    return Vec::new();
                }
                KeyCode::Enter => {
                    return match self.target(store) {
                        Some(id) => {
                            self.detail = Some(id.clone());
                            vec![Cmd::Effect(Effect::Fetch(Fetch::TaskDetail(id)))]
                        }
                        None => match self.target_schedule(store) {
                            Some(s) => {
                                let id = s.id.clone();
                                self.schedule_detail = Some(id.clone());
                                vec![Cmd::Effect(Effect::Fetch(Fetch::ScheduleDetail(id)))]
                            }
                            None => Vec::new(),
                        },
                    };
                }
                _ => {}
            }
        }
        // keys that act on the open detail, whichever pane it came from
        match k.code {
            KeyCode::Char('n') if !self.any_detail() => {
                vec![Cmd::Input(InputPurpose::NewTask, String::new())]
            }
            KeyCode::Char('a') => match self.target(store) {
                Some(task) => vec![Cmd::Input(InputPurpose::Assign { task }, String::new())],
                None => Vec::new(),
            },
            KeyCode::Char('x') => {
                let Some(id) = self.target(store) else {
                    return Vec::new();
                };
                if store.tasks.get(&id).is_some_and(|t| t.state.is_terminal()) {
                    return vec![Cmd::Toast("job already closed".into())];
                }
                if confirming.as_ref() == Some(&id) {
                    vec![super::cancel_task(&id)]
                } else {
                    self.confirm_cancel = Some(id.clone());
                    vec![Cmd::Toast(format!(
                        "press x again to cancel …{}",
                        short(&id)
                    ))]
                }
            }
            KeyCode::Char('p') if self.pane == Pane::Schedules => match self.target_schedule(store)
            {
                Some(s) if s.enabled => vec![Cmd::Effect(Effect::Api(Api::SchedulePause {
                    id: s.id.clone(),
                }))],
                Some(s) => vec![Cmd::Effect(Effect::Api(Api::ScheduleResume {
                    id: s.id.clone(),
                }))],
                None => Vec::new(),
            },
            KeyCode::Char('R') if self.pane == Pane::Schedules => match self.target_schedule(store)
            {
                Some(s) => vec![Cmd::Effect(Effect::Api(Api::ScheduleRun {
                    id: s.id.clone(),
                }))],
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, ctx: &Ctx) {
        let (lists, detail) = if self.any_detail() {
            let [l, d] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(area);
            (l, Some(d))
        } else {
            (area, None)
        };
        let areas: [Rect; 4] = Layout::vertical([
            Constraint::Percentage(30),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .areas(lists);
        for (i, pane) in PANES.iter().enumerate() {
            self.render_pane(f, areas[i], *pane, ctx);
        }
        let Some(d) = detail else { return };
        if let Some(id) = &self.detail {
            render_detail(f, d, id, self.detail_scroll, ctx);
        } else if let Some(id) = &self.schedule_detail {
            render_schedule_detail(f, d, id, self.detail_scroll, ctx);
        }
    }

    fn render_pane(&mut self, f: &mut Frame, area: Rect, pane: Pane, ctx: &Ctx) {
        let store = ctx.store;
        let i = Self::pane_idx(pane);
        let focused = pane == self.pane && !self.any_detail();
        let (title, header, widths, rows, sel): (String, Vec<&str>, Vec<Constraint>, Vec<Row>, _) =
            match pane {
                Pane::Schedules => {
                    let ids: Vec<&ScheduleId> = store.schedules.iter().map(|s| &s.id).collect();
                    (
                        format!(" schedules ({}) ", store.schedules.len()),
                        vec!["CRON", "ZONE", "TARGET", "NEXT", "LAST", "STATE", "TITLE"],
                        vec![
                            Constraint::Length(13),
                            Constraint::Length(19),
                            Constraint::Length(10),
                            Constraint::Length(22),
                            Constraint::Length(5),
                            Constraint::Length(7),
                            Constraint::Min(8),
                        ],
                        store
                            .schedules
                            .iter()
                            .map(|s| schedule_row(s, ctx))
                            .collect(),
                        super::index_of(&ids, &self.sel_schedule.as_ref()),
                    )
                }
                _ => {
                    let tasks = self.pane_tasks(store, pane);
                    let ids: Vec<&TaskId> = tasks.iter().map(|t| &t.id).collect();
                    let sel = super::index_of(&ids, &self.sel[i].as_ref());
                    let (title, header, widths, rows) = match pane {
                        Pane::Owned => (
                            format!(" owned ({}) ", tasks.len()),
                            vec!["ID", "OWNER", "STATE", "LEASE", "TRY", "TITLE"],
                            vec![
                                Constraint::Length(6),
                                Constraint::Length(12),
                                Constraint::Length(14),
                                Constraint::Length(8),
                                Constraint::Length(5),
                                Constraint::Min(8),
                            ],
                            tasks.iter().map(|t| owned_row(t, ctx)).collect(),
                        ),
                        Pane::Queue => (
                            format!(" queue ({}) ", tasks.len()),
                            vec!["ID", "PRI", "WAITING FOR", "TAGS", "AGE", "TITLE"],
                            vec![
                                Constraint::Length(6),
                                Constraint::Length(3),
                                Constraint::Length(24),
                                Constraint::Length(14),
                                Constraint::Length(4),
                                Constraint::Min(8),
                            ],
                            tasks.iter().map(|t| queue_row(t, ctx)).collect(),
                        ),
                        _ => (
                            format!(" recent ({}) ", tasks.len()),
                            vec!["ID", "STATE", "OWNER", "WHEN", "TITLE"],
                            vec![
                                Constraint::Length(6),
                                Constraint::Length(10),
                                Constraint::Length(12),
                                Constraint::Length(5),
                                Constraint::Min(8),
                            ],
                            tasks.iter().map(|t| recent_row(t, ctx)).collect(),
                        ),
                    };
                    (title, header, widths, rows, sel)
                }
            };
        let border = if focused {
            Style::new().fg(Color::Cyan)
        } else {
            super::dim()
        };
        let block = Block::bordered().title(title).border_style(border);
        if rows.is_empty() {
            let empty = match pane {
                Pane::Owned => "no owned jobs",
                Pane::Queue => "queue empty — `n` queues a job",
                Pane::Recent => "nothing closed yet",
                Pane::Schedules => "no schedules",
            };
            f.render_widget(
                Paragraph::new(Span::styled(empty, super::dim())).block(block),
                area,
            );
            return;
        }
        self.tables[i].select(if focused { sel } else { None });
        let table = Table::new(rows, widths)
            .header(Row::new(header).style(super::header()))
            .row_highlight_style(super::selected())
            .block(block);
        f.render_stateful_widget(table, area, &mut self.tables[i]);
    }
}

fn state_style(s: TaskState) -> Style {
    match s {
        TaskState::Working => Style::new().fg(Color::Green),
        TaskState::Assigned => Style::new().fg(Color::Blue),
        TaskState::InputRequired => Style::new().fg(Color::Yellow),
        TaskState::Completed => Style::new().fg(Color::Cyan),
        TaskState::Failed => Style::new().fg(Color::Red),
        _ => super::dim(),
    }
}

/// Live lease cell: countdown, `held` while waiting on input, `expired`
/// (requeues on the next sweep).
pub fn lease_cell(t: &Task, now: i64) -> (String, Style) {
    if t.state == TaskState::InputRequired {
        return ("held".into(), Style::new().fg(Color::Yellow));
    }
    match t.lease_expires_at {
        Some(e) if e <= now => ("expired".into(), Style::new().fg(Color::Red)),
        Some(e) if e - now < 15_000 => (fmt::countdown(e - now), Style::new().fg(Color::Yellow)),
        Some(e) => (fmt::countdown(e - now), Style::new()),
        None => ("—".into(), Style::new()),
    }
}

fn owned_row<'a>(t: &'a Task, ctx: &Ctx<'a>) -> Row<'a> {
    let owner = t.owner_id.as_ref().map_or("—", |o| ctx.store.name_of(o));
    let (lease, lease_style) = lease_cell(t, ctx.now);
    Row::new(vec![
        Cell::from(short(&t.id)),
        Cell::from(owner.to_string()),
        Cell::from(Span::styled(fmt::wire(t.state), state_style(t.state))),
        Cell::from(Span::styled(lease, lease_style)),
        Cell::from(format!("{}/{}", t.attempt_count, t.max_attempts)),
        Cell::from(t.title.as_str()),
    ])
}

/// Why a queued job is waiting: its reservation, or nothing.
pub fn waiting_for(t: &Task, ctx: &Ctx) -> String {
    let target = t.target_agent_id.as_ref().map(|a| ctx.store.name_of(a));
    let held = t.not_before.filter(|nb| *nb > ctx.now);
    match (target, held) {
        (Some(who), Some(nb)) => format!("→{who} in {}", fmt::countdown(nb - ctx.now)),
        (Some(who), None) => format!("→{who} when available"),
        (None, Some(nb)) => format!("not before {}", ctx.tz.clock(nb, false)),
        (None, None) => "—".into(),
    }
}

fn queue_row<'a>(t: &'a Task, ctx: &Ctx<'a>) -> Row<'a> {
    Row::new(vec![
        Cell::from(short(&t.id)),
        Cell::from(t.priority.to_string()),
        Cell::from(waiting_for(t, ctx)),
        Cell::from(if t.tags.is_empty() {
            "—".into()
        } else {
            t.tags.join(",")
        }),
        Cell::from(fmt::age(ctx.now - t.created_at)),
        Cell::from(t.title.as_str()),
    ])
}

fn recent_row<'a>(t: &'a Task, ctx: &Ctx<'a>) -> Row<'a> {
    let owner = t.owner_id.as_ref().map_or("—", |o| ctx.store.name_of(o));
    Row::new(vec![
        Cell::from(short(&t.id)),
        Cell::from(Span::styled(fmt::wire(t.state), state_style(t.state))),
        Cell::from(owner.to_string()),
        Cell::from(fmt::age(ctx.now - t.updated_at)),
        Cell::from(t.title.as_str()),
    ])
}

fn schedule_row<'a>(s: &'a Schedule, ctx: &Ctx<'a>) -> Row<'a> {
    let next = match (s.enabled, s.next_run_at) {
        (true, Some(n)) => format!(
            "{} ({})",
            ctx.tz.clock(n, true).get(5..).unwrap_or_default(),
            fmt::countdown(n - ctx.now)
        ),
        _ => "—".into(),
    };
    let target = s
        .target_agent_id
        .as_ref()
        .map_or("queue".to_string(), |a| ctx.store.name_of(a).to_string());
    let (state, style) = if s.enabled {
        ("active", Style::new().fg(Color::Green))
    } else {
        ("paused", Style::new().fg(Color::Yellow))
    };
    Row::new(vec![
        Cell::from(s.cron.as_str()),
        Cell::from(s.timezone.as_str()),
        Cell::from(target),
        Cell::from(next),
        Cell::from(s.last_run_at.map_or("—".into(), |l| fmt::age(ctx.now - l))),
        Cell::from(Span::styled(state, style)),
        Cell::from(s.title.as_str()),
    ])
}

/// One trail event, operator wording (matches `tower task show`).
pub fn trail_line(e: &Event, store: &Store) -> String {
    let p = &e.payload;
    let name = |v: &serde_json::Value| -> String {
        v.as_str()
            .map(|id| store.name_of(&id.into()).to_string())
            .unwrap_or_else(|| "?".into())
    };
    match e.kind.as_str() {
        "task.created" => "created".into(),
        "task.assigned" => format!(
            "assigned to {} by {}",
            name(&p["owner_id"]),
            p["by"].as_str().unwrap_or("?")
        ),
        "task.reserved" if p["target"].is_null() => "reservation dropped (target removed)".into(),
        "task.reserved" => format!("reserved for {}", name(&p["target"])),
        "task.status" if p["released_by"].is_string() => {
            let mut s = format!("released by {}", name(&p["released_by"]));
            if let Some(r) = p["reason"].as_str() {
                s.push_str(&format!(" — \"{r}\""));
            }
            s
        }
        "task.status" => match p["state"].as_str() {
            Some("working") => format!("working ({})", name(&p["owner_id"])),
            Some(s) => format!("status: {s}"),
            None => "status".into(),
        },
        "task.leased_out" => format!(
            "lease expired ({}) → {}, attempt {}/{}",
            name(&p["prior_owner"]),
            p["state"].as_str().unwrap_or("?"),
            p["attempt_count"],
            p["max_attempts"]
        ),
        "task.completed" => format!("completed by {}", name(&p["owner_id"])),
        "task.failed" => format!("failed: {}", p["result"]),
        other => other.to_string(),
    }
}

fn render_detail(f: &mut Frame, area: Rect, id: &TaskId, scroll: usize, ctx: &Ctx) {
    let block = Block::bordered().title(" job — esc closes · j/k scrolls ");
    let d = match &ctx.store.task_detail {
        Some(d) if &d.task.id == id => d,
        _ => {
            f.render_widget(
                Paragraph::new(Span::styled("loading…", super::dim())).block(block),
                area,
            );
            return;
        }
    };
    // the row in the store is fresher than the detail snapshot
    let t = ctx.store.tasks.get(id).unwrap_or(&d.task);
    let mut state = vec![
        Span::raw("state    "),
        Span::styled(fmt::wire(t.state), state_style(t.state)),
    ];
    if let Some(o) = &t.owner_id {
        state.push(Span::raw(format!("  owner {}", ctx.store.name_of(o))));
        if !t.state.is_terminal() {
            let (lease, style) = lease_cell(t, ctx.now);
            state.push(Span::raw("  lease "));
            state.push(Span::styled(lease, style));
        }
    } else if t.state == TaskState::Queued {
        state.push(Span::raw(format!("  {}", waiting_for(t, ctx))));
    }
    let mut lines = vec![
        Line::styled(t.title.clone(), super::header()),
        Line::styled(t.id.0.clone(), super::dim()),
        Line::from(state),
        Line::raw(format!(
            "attempts {}/{} · priority {}{}",
            t.attempt_count,
            t.max_attempts,
            t.priority,
            if t.tags.is_empty() {
                String::new()
            } else {
                format!(" · tags {}", t.tags.join(","))
            }
        )),
        Line::raw(format!("created  {}", ctx.tz.clock(t.created_at, true))),
    ];
    if let Some(desc) = &t.description {
        lines.push(Line::raw(format!("about    {desc}")));
    }
    if let Some(r) = &t.result {
        lines.push(Line::raw(format!("result   {r}")));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("trail", super::header()));
    for e in &d.trail {
        lines.push(Line::from(vec![
            Span::styled(format!("  {}  ", ctx.tz.clock(e.ts, false)), super::dim()),
            Span::raw(trail_line(e, ctx.store)),
        ]));
    }
    if !d.messages.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("messages", super::header()));
        for m in &d.messages {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}  ", ctx.tz.clock(m.created_at, false)),
                    super::dim(),
                ),
                Span::raw(format!(
                    "{} → {} {}: {}",
                    m.from_id,
                    m.to_id,
                    fmt::wire(m.kind),
                    fmt::summary(&m.parts)
                )),
            ]));
        }
    }
    scroll_paragraph(f, area, block, lines, scroll);
}

/// A schedule's definition and the jobs it fired, scrollable like the job
/// detail.
fn render_schedule_detail(f: &mut Frame, area: Rect, id: &ScheduleId, scroll: usize, ctx: &Ctx) {
    let block = Block::bordered().title(" schedule — esc closes · j/k scrolls ");
    let d = match &ctx.store.schedule_detail {
        Some(d) if &d.schedule.id == id => d,
        _ => {
            f.render_widget(
                Paragraph::new(Span::styled("loading…", super::dim())).block(block),
                area,
            );
            return;
        }
    };
    // the list row is fresher than the detail snapshot
    let s = ctx
        .store
        .schedules
        .iter()
        .find(|s| s.id == d.schedule.id)
        .unwrap_or(&d.schedule);
    let mut lines = vec![
        Line::styled(s.title.clone(), super::header()),
        Line::styled(s.id.0.clone(), super::dim()),
        Line::raw(format!(
            "cron     {} ({})",
            s.cron,
            if s.enabled { "active" } else { "paused" }
        )),
        Line::raw(format!(
            "target   {}",
            s.target_agent_id
                .as_ref()
                .map_or("queue".to_string(), |a| ctx.store.name_of(a).to_string())
        )),
        Line::raw(format!(
            "priority {} · lease {}s · attempts {}{}",
            s.priority,
            s.lease_s,
            s.max_attempts,
            if s.tags.is_empty() {
                String::new()
            } else {
                format!(" · tags {}", s.tags.join(","))
            }
        )),
        Line::raw(format!("created  {}", ctx.tz.clock(s.created_at, true))),
    ];
    if let Some(nb) = s.next_run_at {
        lines.push(Line::raw(format!("next     {}", ctx.tz.clock(nb, true))));
    }
    if let Some(lr) = s.last_run_at {
        lines.push(Line::raw(format!("last     {}", ctx.tz.clock(lr, true))));
    }
    if let Some(desc) = &s.description {
        lines.push(Line::raw(""));
        lines.push(Line::raw(format!("about    {desc}")));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("recent jobs", super::header()));
    for t in &d.jobs {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}  ", ctx.tz.clock(t.created_at, false)),
                super::dim(),
            ),
            Span::styled(fmt::wire(t.state), state_style(t.state)),
            Span::raw(format!("  {}", t.title)),
        ]));
    }
    scroll_paragraph(f, area, block, lines, scroll);
}

/// Bottom-anchored scroll: 0 = tail (newest content), up = older lines.
/// Clamped to the wrapped line count so g can jump past the top harmlessly.
fn scroll_paragraph(
    f: &mut Frame,
    area: Rect,
    block: Block<'_>,
    lines: Vec<Line<'_>>,
    scroll: usize,
) {
    let inner = block.inner(area);
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(block);
    let total = paragraph.line_count(inner.width);
    let max_up = total.saturating_sub(inner.height as usize);
    let top = max_up.saturating_sub(scroll.min(max_up));
    f.render_widget(paragraph.scroll((top as u16, 0)), area);
}
