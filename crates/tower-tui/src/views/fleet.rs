//! Fleet view (plan T2.1, D§11): queue banner + agents table with
//! state filter, machine filter and sort.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use tower_core::{Agent, AgentId, AgentState, MessageStatus};

use super::{Cmd, Ctx};
use crate::app::{Api, Effect, InputPurpose};
use crate::store::Store;

pub const HINTS: &str =
    "enter open · i prompt · x interrupt · o herdr · f state · m machine · s sort";
pub const HELP: &[&str] = &[
    "enter   open agent detail",
    "i       prompt the selected agent",
    "x       interrupt it (ctrl+c)",
    "o       show its pane in herdr",
    "f       cycle state filter (working, idle, blocked, done, dead, all)",
    "m       cycle machine filter",
    "s       cycle sort (name, state, machine)",
    "c       clear filters",
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    /// Attention first: blocked, working, launching, idle, done, unknown, dead.
    State,
    Machine,
}

impl Sort {
    fn label(&self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::State => "state",
            Sort::Machine => "machine",
        }
    }
}

/// State filter cycle (`f`).
const STATE_CYCLE: [AgentState; 5] = [
    AgentState::Working,
    AgentState::Idle,
    AgentState::Blocked,
    AgentState::Done,
    AgentState::Dead,
];

fn attention(s: AgentState) -> u8 {
    match s {
        AgentState::Blocked => 0,
        AgentState::Working => 1,
        AgentState::Launching => 2,
        AgentState::Idle => 3,
        AgentState::Done => 4,
        AgentState::Dead => 6,
        _ => 5,
    }
}

#[derive(Debug, Default)]
pub struct Fleet {
    pub sel: Option<AgentId>,
    pub state: Option<AgentState>,
    /// Machine name.
    pub machine: Option<String>,
    pub sort: Sort,
    table: TableState,
}

impl Fleet {
    /// Filtered + sorted rows.
    pub fn rows<'a>(&self, store: &'a Store) -> Vec<&'a Agent> {
        let mut rows: Vec<&Agent> = store
            .agents
            .iter()
            .filter(|a| self.state.is_none_or(|s| a.state == s))
            .filter(|a| {
                self.machine
                    .as_deref()
                    .is_none_or(|m| store.machine_name(&a.machine_id) == m)
            })
            .collect();
        match self.sort {
            Sort::Name => {} // store keeps agents by name
            Sort::State => rows.sort_by_key(|a| attention(a.state)),
            Sort::Machine => rows.sort_by(|a, b| {
                store
                    .machine_name(&a.machine_id)
                    .cmp(store.machine_name(&b.machine_id))
            }),
        }
        rows
    }

    fn selected<'a>(&self, store: &'a Store) -> Option<&'a Agent> {
        let rows = self.rows(store);
        let ids: Vec<&AgentId> = rows.iter().map(|a| &a.id).collect();
        super::index_of(&ids, &self.sel.as_ref()).map(|i| rows[i])
    }

    pub fn key(&mut self, k: KeyEvent, ctx: &Ctx) -> Vec<Cmd> {
        if let Some(n) = super::nav(&k) {
            let ids: Vec<AgentId> = self.rows(ctx.store).iter().map(|a| a.id.clone()).collect();
            self.sel = super::step(&ids, &self.sel, n);
            return Vec::new();
        }
        match k.code {
            KeyCode::Char('f') => {
                self.state = match self.state {
                    None => Some(STATE_CYCLE[0]),
                    Some(s) => STATE_CYCLE
                        .iter()
                        .position(|c| *c == s)
                        .and_then(|i| STATE_CYCLE.get(i + 1))
                        .copied(),
                };
                Vec::new()
            }
            KeyCode::Char('m') => {
                let mut names: Vec<&str> = ctx
                    .store
                    .agents
                    .iter()
                    .map(|a| ctx.store.machine_name(&a.machine_id))
                    .collect();
                names.sort_unstable();
                names.dedup();
                self.machine = match self.machine.as_deref() {
                    None => names.first().map(|s| s.to_string()),
                    Some(m) => names
                        .iter()
                        .position(|n| *n == m)
                        .and_then(|i| names.get(i + 1))
                        .map(|s| s.to_string()),
                };
                Vec::new()
            }
            KeyCode::Char('s') => {
                self.sort = match self.sort {
                    Sort::Name => Sort::State,
                    Sort::State => Sort::Machine,
                    Sort::Machine => Sort::Name,
                };
                Vec::new()
            }
            KeyCode::Char('c') => {
                self.state = None;
                self.machine = None;
                Vec::new()
            }
            _ => {
                let Some(a) = self.selected(ctx.store) else {
                    return Vec::new();
                };
                agent_action(k, a)
            }
        }
    }

    /// `:filter state=<s> machine=<m>`; no args clears.
    pub fn set_filter(&mut self, args: &[&str], store: &Store) -> Result<(), String> {
        let (mut state, mut machine) = (None, None);
        for a in args {
            match a.split_once('=') {
                Some(("state", s)) => {
                    state = Some(
                        serde_json::from_value::<AgentState>(s.into())
                            .map_err(|_| format!("unknown state {s}"))?,
                    )
                }
                Some(("machine", m)) => {
                    if !store.machines.iter().any(|x| x.name == m) {
                        return Err(format!("unknown machine {m}"));
                    }
                    machine = Some(m.to_string())
                }
                _ => return Err(format!("filter takes state=<s> machine=<m>, not {a}")),
            }
        }
        self.state = state;
        self.machine = machine;
        Ok(())
    }

    pub fn set_sort(&mut self, key: &str) -> Result<(), String> {
        self.sort = match key {
            "name" => Sort::Name,
            "state" => Sort::State,
            "machine" => Sort::Machine,
            _ => return Err(format!("sort by name, state or machine — not {key}")),
        };
        Ok(())
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, ctx: &Ctx) {
        let [banner, body] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(area);
        self.render_banner(f, banner, ctx.store);

        let rows = self.rows(ctx.store);
        if rows.is_empty() {
            let msg = if ctx.store.agents.is_empty() {
                "no agents — `:spawn <name> [kind]` starts one"
            } else {
                "no agents match the filter — `c` clears it"
            };
            f.render_widget(Paragraph::new(Span::styled(msg, super::dim())), body);
            return;
        }
        let ids: Vec<&AgentId> = rows.iter().map(|a| &a.id).collect();
        self.table.select(super::index_of(&ids, &self.sel.as_ref()));
        let table_rows: Vec<Row> = rows.iter().map(|a| row(a, ctx.store)).collect();
        let table = Table::new(
            table_rows,
            [
                Constraint::Length(2),
                Constraint::Length(14),
                Constraint::Length(9),
                Constraint::Length(8),
                Constraint::Length(10),
                Constraint::Length(32),
                Constraint::Min(8),
            ],
        )
        .header(
            Row::new(["", "NAME", "MACHINE", "KIND", "STATE", "TASK", "NOTE"])
                .style(super::header()),
        )
        .row_highlight_style(super::selected());
        f.render_stateful_widget(table, body, &mut self.table);
    }

    fn render_banner(&self, f: &mut Frame, area: Rect, store: &Store) {
        let q = store.queue_counts();
        let blocked = if q.blocked > 0 {
            Style::new().fg(Color::Yellow)
        } else {
            Style::new()
        };
        let mut spans = vec![
            Span::raw(format!("{} queued · {} working · ", q.queued, q.working)),
            Span::styled(format!("{} blocked", q.blocked), blocked),
        ];
        let mut filters = Vec::new();
        if let Some(s) = self.state {
            filters.push(format!("state={}", s.as_str()));
        }
        if let Some(m) = &self.machine {
            filters.push(format!("machine={m}"));
        }
        if self.sort != Sort::Name {
            filters.push(format!("sort={}", self.sort.label()));
        }
        if !filters.is_empty() {
            spans.push(Span::styled(
                format!("   [{}]", filters.join(" · ")),
                super::dim(),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

/// Keys shared by the fleet row and the agent detail.
pub fn agent_action(k: KeyEvent, a: &Agent) -> Vec<Cmd> {
    match k.code {
        KeyCode::Enter => vec![Cmd::OpenAgent(a.id.clone())],
        KeyCode::Char('i') => vec![Cmd::Input(
            InputPurpose::Prompt {
                agent: a.name.clone(),
            },
            String::new(),
        )],
        KeyCode::Char('x') => vec![Cmd::Effect(Effect::Api(Api::Interrupt {
            agent: a.name.clone(),
        }))],
        KeyCode::Char('o') => vec![Cmd::Effect(Effect::Attach {
            target: a.pane_id.clone().unwrap_or_else(|| a.name.clone()),
        })],
        _ => Vec::new(),
    }
}

fn row<'a>(a: &'a Agent, store: &'a Store) -> Row<'a> {
    let style = super::state_style(a.state);
    let task = store
        .current_task(&a.id)
        .map(|t| crate::fmt::trunc(&t.title, 31))
        .unwrap_or_else(|| "—".into());
    Row::new(vec![
        Cell::from(Span::styled(a.state.glyph(), style)),
        Cell::from(a.name.as_str()),
        Cell::from(store.machine_name(&a.machine_id)),
        Cell::from(a.kind.as_str()),
        Cell::from(Span::styled(a.state.as_str(), style)),
        Cell::from(task),
        Cell::from(note(a, store)),
    ])
}

/// Context note: why it needs you, then how it was set up.
fn note(a: &Agent, store: &Store) -> String {
    let mut parts: Vec<String> = Vec::new();
    let waiting = store
        .inbox
        .iter()
        .filter(|m| m.status == MessageStatus::Pending)
        .find(|m| m.from_id == a.name || m.from_id == a.id.0);
    if let Some(m) = waiting {
        parts.push(format!("awaiting {}", crate::fmt::wire(m.kind)));
    }
    if a.desired_state == tower_core::DesiredState::Stopped {
        parts.push("stopped".into());
    }
    if a.adopted {
        parts.push("adopted".into());
    }
    if let Some(w) = &a.worktree {
        parts.push(w.rsplit('/').next().unwrap_or(w).to_string());
    }
    parts.join(" · ")
}
