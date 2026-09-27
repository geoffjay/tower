//! App state machine (plan T1.1): view router, global keymap, command
//! palette, input line, toasts. Pure — no I/O. Keys, feed messages and
//! fetch results go in; `Effect`s come out for the runtime to execute.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;
use tower_core::{AgentId, MessageId, ScheduleId, TaskId};

use crate::feed::FeedMsg;
use crate::fmt::Tz;
use crate::input::{Edit, LineInput};
use crate::store::{Fetch, Loaded, Store};
use crate::views::{self, Cmd, Ctx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewId {
    Fleet,
    Agent,
    Inbox,
    Tasks,
    Events,
    Machines,
}

impl ViewId {
    fn title(&self) -> &'static str {
        match self {
            ViewId::Fleet => "Fleet",
            ViewId::Agent => "Agent",
            ViewId::Inbox => "Inbox",
            ViewId::Tasks => "Tasks",
            ViewId::Events => "Events",
            ViewId::Machines => "Machines",
        }
    }
}

/// Numbered views (`1`-`5`); agent detail is entered from the fleet.
const NUMBERED: [ViewId; 5] = [
    ViewId::Fleet,
    ViewId::Inbox,
    ViewId::Tasks,
    ViewId::Events,
    ViewId::Machines,
];

/// Work for the runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Fetch(Fetch),
    Api(Api),
    /// Show the agent's pane in herdr (focus when inside herdr, else attach).
    Attach {
        target: String,
    },
}

/// Mutating `/v1` calls (the D§10 verbs the TUI wraps).
#[derive(Debug, Clone, PartialEq)]
pub enum Api {
    Prompt {
        agent: String,
        text: String,
    },
    Interrupt {
        agent: String,
    },
    Respond {
        msg: MessageId,
        approve: Option<bool>,
        text: Option<String>,
    },
    Spawn {
        name: String,
        kind: Option<String>,
        workdir: Option<String>,
    },
    Stop {
        agent: String,
    },
    Ask {
        agent: String,
        text: String,
    },
    TaskCreate {
        title: String,
    },
    TaskAssign {
        task: TaskId,
        agent: String,
        /// Reserve: deliver when the agent is free (D§5.2.2).
        when_available: bool,
    },
    TaskCancel {
        task: TaskId,
    },
    SchedulePause {
        id: ScheduleId,
    },
    ScheduleResume {
        id: ScheduleId,
    },
    ScheduleRun {
        id: ScheduleId,
    },
}

/// What the focused input line is for.
#[derive(Debug, Clone, PartialEq)]
pub enum InputPurpose {
    Palette,
    Prompt { agent: String },
    Reply { msg: MessageId },
    NewTask,
    Assign { task: TaskId },
    EventKind,
    EventSubject,
}

impl InputPurpose {
    fn label(&self) -> String {
        match self {
            InputPurpose::Palette => ":".into(),
            InputPurpose::Prompt { agent } => format!("prompt {agent} › "),
            InputPurpose::Reply { .. } => "reply › ".into(),
            InputPurpose::NewTask => "new job title › ".into(),
            InputPurpose::Assign { .. } => "assign to agent (`<name> later` reserves) › ".into(),
            InputPurpose::EventKind => "filter type › ".into(),
            InputPurpose::EventSubject => "filter subject › ".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conn {
    Connecting,
    Live,
    Down(String),
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: i64,
}

/// Lease renewals (heartbeats) emit no event: sweep `?since=` this often.
const TASK_SWEEP_MS: i64 = 10_000;

pub struct App {
    pub store: Store,
    pub view: ViewId,
    pub fleet: views::fleet::Fleet,
    pub agent: Option<views::agent::AgentView>,
    pub inbox: views::inbox::Inbox,
    pub tasks: views::tasks::Tasks,
    pub events: views::events::Events,
    pub machines: views::machines::Machines,
    pub input: Option<(InputPurpose, LineInput)>,
    pub help: bool,
    pub toast: Option<Toast>,
    pub conn: Conn,
    pub now: i64,
    pub tz: Tz,
    pub quit: bool,
    /// In-flight fetches → "an event arrived meanwhile; fetch again".
    inflight: HashMap<Fetch, bool>,
    last_sweep: i64,
}

impl App {
    pub fn new(now: i64, tz: Tz) -> Self {
        Self {
            store: Store::default(),
            view: ViewId::Fleet,
            fleet: Default::default(),
            agent: None,
            inbox: Default::default(),
            tasks: Default::default(),
            events: Default::default(),
            machines: Default::default(),
            input: None,
            help: false,
            toast: None,
            conn: Conn::Connecting,
            now,
            tz,
            quit: false,
            inflight: HashMap::new(),
            last_sweep: now,
        }
    }

    // ---- inputs ---------------------------------------------------------

    pub fn on_feed(&mut self, m: FeedMsg) -> Vec<Effect> {
        match m {
            FeedMsg::Connected => {
                self.conn = Conn::Live;
                let mut f = vec![Fetch::All];
                f.extend(self.view_fetches());
                self.request_all(f)
            }
            FeedMsg::Down(e) => {
                self.conn = Conn::Down(e);
                Vec::new()
            }
            FeedMsg::Events(events) => {
                let mut wanted = Vec::new();
                for e in events {
                    for f in self.store.apply(e) {
                        if self.wanted(&f) && !wanted.contains(&f) {
                            wanted.push(f);
                        }
                    }
                }
                self.request_all(wanted)
            }
        }
    }

    pub fn on_loaded(&mut self, f: Fetch, r: Result<Vec<Loaded>, String>) -> Vec<Effect> {
        let dirty = self.inflight.remove(&f).unwrap_or(false);
        match r {
            Ok(loaded) => {
                for l in loaded {
                    self.store.load(l);
                }
            }
            Err(e) => {
                // a vanished agent/task is routine (removed meanwhile)
                if !e.starts_with("not_found") {
                    self.toast_err(format!("load failed: {e}"));
                }
            }
        }
        if dirty && self.wanted(&f) {
            self.request(f).into_iter().collect()
        } else {
            Vec::new()
        }
    }

    pub fn on_api_started(&mut self, a: &Api) {
        if let Some(label) = crate::exec::pending_label(a) {
            self.toast_ok(label);
        }
    }

    pub fn on_api(&mut self, r: Result<String, String>) {
        match r {
            Ok(msg) => self.toast_ok(msg),
            Err(e) => self.toast_err(e),
        }
    }

    /// Clock tick (1s): countdowns re-render; the task sweep runs.
    pub fn tick(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        if self.toast.as_ref().is_some_and(|t| t.until <= now) {
            self.toast = None;
        }
        if self.conn == Conn::Live && now - self.last_sweep >= TASK_SWEEP_MS {
            self.last_sweep = now;
            let since = self.store.tasks_updated_at();
            return self.request(Fetch::TasksSince(since)).into_iter().collect();
        }
        Vec::new()
    }

    pub fn on_key(&mut self, k: KeyEvent) -> Vec<Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some((purpose, line)) = self.input.as_mut() {
            return match line.key(k) {
                Edit::Submit(text) => {
                    let purpose = purpose.clone();
                    self.input = None;
                    self.submit(purpose, text.trim())
                }
                Edit::Cancel => {
                    self.input = None;
                    Vec::new()
                }
                Edit::Changed | Edit::Ignored => Vec::new(),
            };
        }
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return Vec::new();
        }
        if self.help {
            if matches!(
                k.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
            ) {
                self.help = false;
            }
            return Vec::new();
        }
        match k.code {
            KeyCode::Char('q') => {
                self.quit = true;
                return Vec::new();
            }
            KeyCode::Char('?') => {
                self.help = true;
                return Vec::new();
            }
            KeyCode::Char(':') => {
                self.input = Some((InputPurpose::Palette, LineInput::default()));
                return Vec::new();
            }
            KeyCode::Tab => return self.cycle(1),
            KeyCode::BackTab => return self.cycle(-1),
            KeyCode::Char(c @ '1'..='5') if !ctrl => {
                let v = NUMBERED[(c as u8 - b'1') as usize];
                return self.show(v);
            }
            _ => {}
        }
        let ctx = Ctx {
            store: &self.store,
            now: self.now,
            tz: self.tz,
        };
        let cmds = match self.view {
            ViewId::Fleet => self.fleet.key(k, &ctx),
            ViewId::Agent => match self.agent.as_mut() {
                Some(a) => a.key(k, &ctx),
                None => Vec::new(),
            },
            ViewId::Inbox => self.inbox.key(k, &ctx),
            ViewId::Tasks => self.tasks.key(k, &ctx),
            ViewId::Events => self.events.key(k, &ctx),
            ViewId::Machines => self.machines.key(k, &ctx),
        };
        self.run(cmds)
    }

    // ---- routing --------------------------------------------------------

    fn tabs(&self) -> Vec<ViewId> {
        let mut t = vec![ViewId::Fleet];
        if self.agent.is_some() {
            t.push(ViewId::Agent);
        }
        t.extend(&NUMBERED[1..]);
        t
    }

    fn cycle(&mut self, dir: isize) -> Vec<Effect> {
        let tabs = self.tabs();
        let i = tabs.iter().position(|v| *v == self.view).unwrap_or(0) as isize;
        let n = tabs.len() as isize;
        self.show(tabs[((i + dir).rem_euclid(n)) as usize])
    }

    fn show(&mut self, v: ViewId) -> Vec<Effect> {
        self.view = v;
        let f = self.view_fetches();
        self.request_all(f)
    }

    /// Fetches the visible view needs fresh (entering it, reconnecting).
    fn view_fetches(&self) -> Vec<Fetch> {
        match self.view {
            ViewId::Agent => match &self.agent {
                Some(a) => vec![Fetch::Output(a.id.clone()), Fetch::History(a.id.clone())],
                None => vec![],
            },
            ViewId::Tasks => self
                .tasks
                .detail
                .clone()
                .map(Fetch::TaskDetail)
                .into_iter()
                .collect(),
            _ => vec![],
        }
    }

    /// View-scoped fetches only matter while that view shows the object.
    fn wanted(&self, f: &Fetch) -> bool {
        match f {
            Fetch::Output(id) | Fetch::History(id) => {
                self.view == ViewId::Agent && self.agent.as_ref().is_some_and(|a| &a.id == id)
            }
            Fetch::TaskDetail(id) => self.tasks.detail.as_ref() == Some(id),
            _ => true,
        }
    }

    fn request(&mut self, f: Fetch) -> Option<Effect> {
        match self.inflight.get_mut(&f) {
            Some(dirty) => {
                *dirty = true;
                None
            }
            None => {
                self.inflight.insert(f.clone(), false);
                Some(Effect::Fetch(f))
            }
        }
    }

    fn request_all(&mut self, fs: Vec<Fetch>) -> Vec<Effect> {
        fs.into_iter().filter_map(|f| self.request(f)).collect()
    }

    fn open_agent(&mut self, id: AgentId) -> Vec<Effect> {
        if self.agent.as_ref().map(|a| &a.id) != Some(&id) {
            self.agent = Some(views::agent::AgentView::new(id));
        }
        self.show(ViewId::Agent)
    }

    fn run(&mut self, cmds: Vec<Cmd>) -> Vec<Effect> {
        let mut out = Vec::new();
        for c in cmds {
            match c {
                Cmd::Effect(Effect::Fetch(f)) => out.extend(self.request(f)),
                Cmd::Effect(e) => out.push(e),
                Cmd::Input(p, prefill) => self.input = Some((p, LineInput::with(&prefill))),
                Cmd::OpenAgent(id) => out.extend(self.open_agent(id)),
                Cmd::Back => {
                    if self.view == ViewId::Agent {
                        out.extend(self.show(ViewId::Fleet));
                    }
                }
                Cmd::Toast(t) => self.toast_ok(t),
            }
        }
        out
    }

    fn toast_ok(&mut self, text: String) {
        self.toast = Some(Toast {
            text,
            error: false,
            until: self.now + 4_000,
        });
    }

    fn toast_err(&mut self, text: String) {
        self.toast = Some(Toast {
            text,
            error: true,
            until: self.now + 8_000,
        });
    }

    // ---- input submission ------------------------------------------------

    fn submit(&mut self, purpose: InputPurpose, text: &str) -> Vec<Effect> {
        if text.is_empty()
            && purpose != InputPurpose::EventKind
            && purpose != InputPurpose::EventSubject
        {
            return Vec::new();
        }
        let api = |a| vec![Effect::Api(a)];
        match purpose {
            InputPurpose::Palette => self.palette(text),
            InputPurpose::Prompt { agent } => api(Api::Prompt {
                agent,
                text: text.into(),
            }),
            InputPurpose::Reply { msg } => api(Api::Respond {
                msg,
                approve: None,
                text: Some(text.into()),
            }),
            InputPurpose::NewTask => api(Api::TaskCreate { title: text.into() }),
            InputPurpose::Assign { task } => {
                let (agent, later) = match text.strip_suffix(" later") {
                    Some(a) => (a.trim(), true),
                    None => (text, false),
                };
                api(Api::TaskAssign {
                    task,
                    agent: agent.into(),
                    when_available: later,
                })
            }
            InputPurpose::EventKind => {
                self.events.kind_filter = text.into();
                Vec::new()
            }
            InputPurpose::EventSubject => {
                self.events.subject_filter = text.into();
                Vec::new()
            }
        }
    }

    /// `:` commands. Unknown input explains itself instead of failing silently.
    fn palette(&mut self, text: &str) -> Vec<Effect> {
        let mut words = text.split_whitespace();
        let verb = words.next().unwrap_or("");
        let rest: Vec<&str> = words.collect();
        let tail = |from: usize| rest.get(from..).map(|r| r.join(" ")).unwrap_or_default();
        let api = |a| vec![Effect::Api(a)];
        match (verb, rest.as_slice()) {
            ("q" | "quit", _) => {
                self.quit = true;
                Vec::new()
            }
            ("help", _) => {
                self.help = true;
                Vec::new()
            }
            ("fleet", _) => self.show(ViewId::Fleet),
            ("inbox", _) => self.show(ViewId::Inbox),
            ("tasks" | "jobs", _) => self.show(ViewId::Tasks),
            ("events", _) => self.show(ViewId::Events),
            ("machines", _) => self.show(ViewId::Machines),
            ("agent", [name]) => match self.store.agent_by_name(name) {
                Some(a) => {
                    let id = a.id.clone();
                    self.open_agent(id)
                }
                None => {
                    self.toast_err(format!("no agent named {name}"));
                    Vec::new()
                }
            },
            ("spawn", [name, ..]) => api(Api::Spawn {
                name: name.to_string(),
                kind: rest.get(1).map(|s| s.to_string()),
                workdir: rest.get(2).map(|s| s.to_string()),
            }),
            ("stop", [name]) => api(Api::Stop {
                agent: name.to_string(),
            }),
            ("prompt", [name, _, ..]) => api(Api::Prompt {
                agent: name.to_string(),
                text: tail(1),
            }),
            ("ask", [name, _, ..]) => api(Api::Ask {
                agent: name.to_string(),
                text: tail(1),
            }),
            ("interrupt", [name]) => api(Api::Interrupt {
                agent: name.to_string(),
            }),
            ("task", [_, ..]) => api(Api::TaskCreate { title: tail(0) }),
            ("assign" | "reserve", [task, agent]) => match self.resolve_task(task) {
                Some(task) => api(Api::TaskAssign {
                    task,
                    agent: agent.to_string(),
                    when_available: verb == "reserve",
                }),
                None => Vec::new(),
            },
            ("cancel", [task]) => match self.resolve_task(task) {
                Some(task) => api(Api::TaskCancel { task }),
                None => Vec::new(),
            },
            ("filter", args) => {
                match self.fleet.set_filter(args, &self.store) {
                    Ok(()) => self.view = ViewId::Fleet,
                    Err(e) => self.toast_err(e),
                }
                Vec::new()
            }
            ("sort", [key]) => {
                match self.fleet.set_sort(key) {
                    Ok(()) => self.view = ViewId::Fleet,
                    Err(e) => self.toast_err(e),
                }
                Vec::new()
            }
            _ => {
                self.toast_err(format!("unknown command `{text}` — `?` lists commands"));
                Vec::new()
            }
        }
    }

    /// A task by full id or a unique prefix/suffix of it.
    fn resolve_task(&mut self, s: &str) -> Option<TaskId> {
        let hits: Vec<&TaskId> = self
            .store
            .tasks
            .keys()
            .filter(|id| id.0 == s || id.0.starts_with(s) || id.0.ends_with(s))
            .collect();
        match hits.as_slice() {
            [one] => Some((*one).clone()),
            [] => {
                self.toast_err(format!("no job matches {s}"));
                None
            }
            _ => {
                self.toast_err(format!("{s} matches {} jobs — type more", hits.len()));
                None
            }
        }
    }

    // ---- rendering ------------------------------------------------------

    pub fn render(&mut self, f: &mut Frame) {
        let [top, body, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(f.area());
        self.render_tabs(f, top);
        let ctx = Ctx {
            store: &self.store,
            now: self.now,
            tz: self.tz,
        };
        match self.view {
            ViewId::Fleet => self.fleet.render(f, body, &ctx),
            ViewId::Agent => {
                if let Some(a) = self.agent.as_mut() {
                    a.render(f, body, &ctx)
                }
            }
            ViewId::Inbox => self.inbox.render(f, body, &ctx),
            ViewId::Tasks => self.tasks.render(f, body, &ctx),
            ViewId::Events => self.events.render(f, body, &ctx),
            ViewId::Machines => self.machines.render(f, body, &ctx),
        }
        self.render_bottom(f, bottom);
        if matches!(self.input, Some((InputPurpose::Palette, _))) {
            self.render_palette_hints(f, body);
        }
        if self.help {
            self.render_help(f, body);
        }
    }

    fn render_tabs(&self, f: &mut Frame, area: Rect) {
        let mut spans = vec![Span::styled(
            " tower ",
            Style::new().add_modifier(Modifier::BOLD),
        )];
        for v in self.tabs() {
            let num = NUMBERED.iter().position(|n| *n == v).map(|i| i + 1);
            let mut label = match num {
                Some(n) => format!(" {n} {}", v.title()),
                None => format!(" {}", v.title()),
            };
            if v == ViewId::Agent {
                if let Some(a) = self.agent.as_ref() {
                    label.push_str(&format!(": {}", self.store.name_of(&a.id)));
                }
            }
            if v == ViewId::Inbox && !self.store.inbox.is_empty() {
                label.push_str(&format!(" ({})", self.store.inbox.len()));
            }
            label.push(' ');
            let style = if v == self.view {
                Style::new().add_modifier(Modifier::REVERSED)
            } else if v == ViewId::Inbox && !self.store.inbox.is_empty() {
                Style::new().fg(Color::Yellow)
            } else {
                Style::new()
            };
            spans.push(Span::styled(label, style));
        }
        let (conn, style) = match &self.conn {
            Conn::Connecting => (
                " ○ connecting".to_string(),
                Style::new().fg(Color::DarkGray),
            ),
            Conn::Live => (" ● live".to_string(), Style::new().fg(Color::Green)),
            Conn::Down(e) => (
                format!(" ○ reconnecting ({})", crate::fmt::trunc(e, 40)),
                Style::new().fg(Color::Red),
            ),
        };
        let left = Line::from(spans);
        let right_w = conn.chars().count() as u16 + 1;
        let [l, r] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(right_w)]).areas(area);
        f.render_widget(Paragraph::new(left), l);
        f.render_widget(Paragraph::new(Span::styled(conn, style)), r);
    }

    fn render_bottom(&self, f: &mut Frame, area: Rect) {
        if let Some((purpose, line)) = &self.input {
            let label = purpose.label();
            let w = label.chars().count();
            // scroll horizontally so the cursor stays visible in long input
            let room = (area.width as usize).saturating_sub(w + 1).max(1);
            let start = line.cursor().saturating_sub(room);
            let shown: String = line.text().chars().skip(start).take(room + 1).collect();
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(label, Style::new().fg(Color::Cyan)),
                    Span::raw(shown),
                ])),
                area,
            );
            let x = area.x + (w + line.cursor() - start) as u16;
            f.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
            return;
        }
        if let Some(t) = &self.toast {
            let style = if t.error {
                Style::new().fg(Color::Red)
            } else {
                Style::new().fg(Color::Green)
            };
            f.render_widget(Paragraph::new(Span::styled(t.text.clone(), style)), area);
            return;
        }
        let hints = match self.view {
            ViewId::Fleet => views::fleet::HINTS,
            ViewId::Agent => views::agent::HINTS,
            ViewId::Inbox => views::inbox::HINTS,
            ViewId::Tasks => self.tasks.hints(),
            ViewId::Events => views::events::HINTS,
            ViewId::Machines => views::machines::HINTS,
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{hints} · : command · ? help · q quit"),
                views::dim(),
            )),
            area,
        );
    }

    fn render_palette_hints(&self, f: &mut Frame, body: Rect) {
        let typed = self
            .input
            .as_ref()
            .map(|(_, l)| l.text().split_whitespace().next().unwrap_or("").to_string())
            .unwrap_or_default();
        let hits: Vec<&(&str, &str)> = PALETTE
            .iter()
            .filter(|(usage, _)| usage.starts_with(typed.as_str()))
            .collect();
        if hits.is_empty() {
            return;
        }
        let h = (hits.len() as u16 + 2).min(body.height);
        let area = Rect {
            x: body.x,
            y: body.bottom().saturating_sub(h),
            width: body.width.min(64),
            height: h,
        };
        let lines: Vec<Line> = hits
            .iter()
            .map(|(usage, what)| {
                Line::from(vec![
                    Span::styled(format!("{usage:<28}"), Style::new().fg(Color::Cyan)),
                    Span::styled(*what, views::dim()),
                ])
            })
            .collect();
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(lines).block(Block::bordered().title(" commands ")),
            area,
        );
    }

    fn render_help(&self, f: &mut Frame, body: Rect) {
        let mut lines = vec![
            Line::styled("global", views::header()),
            Line::raw("  tab / shift-tab   cycle views        1-5  jump to view"),
            Line::raw("  j k g G ctrl-d/u  move               enter open · esc back"),
            Line::raw("  :                 command palette    q / ctrl-c  quit"),
            Line::raw(""),
            Line::styled(
                format!("{} view", self.view.title().to_lowercase()),
                views::header(),
            ),
        ];
        let view_help: &[&str] = match self.view {
            ViewId::Fleet => views::fleet::HELP,
            ViewId::Agent => views::agent::HELP,
            ViewId::Inbox => views::inbox::HELP,
            ViewId::Tasks => views::tasks::HELP,
            ViewId::Events => views::events::HELP,
            ViewId::Machines => views::machines::HELP,
        };
        lines.extend(view_help.iter().map(|l| Line::raw(format!("  {l}"))));
        lines.push(Line::raw(""));
        lines.push(Line::styled("commands", views::header()));
        lines.extend(
            PALETTE
                .iter()
                .map(|(u, w)| Line::raw(format!("  :{u:<27} {w}"))),
        );
        let w = body.width.min(78);
        let h = (lines.len() as u16 + 2).min(body.height);
        let area = Rect {
            x: body.x + (body.width - w) / 2,
            y: body.y + (body.height - h) / 2,
            width: w,
            height: h,
        };
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(" help — esc closes ")),
            area,
        );
    }
}

/// Palette commands: (usage, what).
pub const PALETTE: &[(&str, &str)] = &[
    (
        "fleet | inbox | tasks",
        "switch view (also events, machines)",
    ),
    ("agent <name>", "open agent detail"),
    ("spawn <name> [kind] [dir]", "spawn an agent"),
    ("stop <name>", "stop an agent's session"),
    ("prompt <name> <text>", "send a prompt"),
    ("ask <name> <text>", "ask an agent a question"),
    ("interrupt <name>", "ctrl+c the agent"),
    ("task <title>", "queue a job"),
    ("assign <job> <agent>", "assign a job (id prefix ok)"),
    ("reserve <job> <agent>", "deliver when the agent is free"),
    ("cancel <job>", "cancel a job"),
    (
        "filter state=<s> machine=<m>",
        "filter the fleet (bare: clear)",
    ),
    ("sort name|state|machine", "sort the fleet"),
    ("quit", "exit"),
];
