//! Inbox (plan T3.1, D§11): pending questions/approvals addressed to me,
//! with deadlines; reply inline — `y`/`n`, or free text for questions.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;
use tower_core::{Message, MessageId, MessageKind};

use super::{Cmd, Ctx};
use crate::app::InputPurpose;
use crate::fmt;

pub const HINTS: &str = "y approve/yes · n deny/no · r reply";
pub const HELP: &[&str] = &[
    "y       approve (approval) or answer \"yes\" (question)",
    "n       deny (approval) or answer \"no\" (question)",
    "r/enter answer a question with free text",
    "o       show the asking agent's pane in herdr",
];

#[derive(Debug, Default)]
pub struct Inbox {
    pub sel: Option<MessageId>,
    table: TableState,
}

impl Inbox {
    fn selected<'a>(&self, ctx: &Ctx<'a>) -> Option<&'a Message> {
        let ids: Vec<&MessageId> = ctx.store.inbox.iter().map(|m| &m.id).collect();
        super::index_of(&ids, &self.sel.as_ref()).map(|i| &ctx.store.inbox[i])
    }

    pub fn key(&mut self, k: KeyEvent, ctx: &Ctx) -> Vec<Cmd> {
        if let Some(n) = super::nav(&k) {
            let ids: Vec<MessageId> = ctx.store.inbox.iter().map(|m| m.id.clone()).collect();
            self.sel = super::step(&ids, &self.sel, n);
            return Vec::new();
        }
        let Some(m) = self.selected(ctx) else {
            return Vec::new();
        };
        let approval = m.kind == MessageKind::Approval;
        match k.code {
            KeyCode::Char('y') if approval => vec![super::respond(&m.id, Some(true), None)],
            KeyCode::Char('n') if approval => vec![super::respond(&m.id, Some(false), None)],
            KeyCode::Char('y') => vec![super::respond(&m.id, None, Some("yes".into()))],
            KeyCode::Char('n') => vec![super::respond(&m.id, None, Some("no".into()))],
            KeyCode::Char('r') | KeyCode::Enter if approval => {
                vec![Cmd::Toast("approvals take y (approve) or n (deny)".into())]
            }
            KeyCode::Char('r') | KeyCode::Enter => vec![Cmd::Input(
                InputPurpose::Reply { msg: m.id.clone() },
                String::new(),
            )],
            KeyCode::Char('o') => match ctx.store.agent_by_name(&m.from_id) {
                Some(a) => super::fleet::agent_action(k, a),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, ctx: &Ctx) {
        let msgs = &ctx.store.inbox;
        if msgs.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "inbox empty — agents' questions and approvals land here",
                    super::dim(),
                )),
                area,
            );
            return;
        }
        // the list takes what its rows need (up to a third); the detail —
        // with the captured screen — gets the rest
        let list_h = (msgs.len() as u16 + 1).min((area.height / 3).max(4));
        let [list, detail] =
            Layout::vertical([Constraint::Length(list_h + 1), Constraint::Min(4)]).areas(area);
        let ids: Vec<&MessageId> = msgs.iter().map(|m| &m.id).collect();
        let idx = super::index_of(&ids, &self.sel.as_ref());
        self.table.select(idx);
        let rows: Vec<Row> = msgs.iter().map(|m| row(m, ctx)).collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(10),
                Constraint::Length(5),
                Constraint::Length(8),
                Constraint::Min(10),
            ],
        )
        .header(Row::new(["FROM", "KIND", "AGE", "EXPIRES", "SUMMARY"]).style(super::header()))
        .row_highlight_style(super::selected());
        f.render_stateful_widget(table, list, &mut self.table);
        if let Some(i) = idx {
            render_detail(f, detail, &msgs[i], ctx);
        }
    }
}

fn expires(m: &Message, now: i64) -> (String, Style) {
    match m.deadline_at {
        Some(d) if d - now < 60_000 => (fmt::countdown(d - now), Style::new().fg(Color::Red)),
        Some(d) => (fmt::countdown(d - now), Style::new()),
        None => ("—".into(), Style::new()),
    }
}

fn row<'a>(m: &'a Message, ctx: &Ctx) -> Row<'a> {
    let kind_style = if m.kind == MessageKind::Approval {
        Style::new().fg(Color::Yellow)
    } else {
        Style::new().fg(Color::Cyan)
    };
    let (exp, exp_style) = expires(m, ctx.now);
    Row::new(vec![
        Cell::from(m.from_id.as_str()),
        Cell::from(Span::styled(fmt::wire(m.kind), kind_style)),
        Cell::from(fmt::age(ctx.now - m.created_at)),
        Cell::from(Span::styled(exp, exp_style)),
        Cell::from(fmt::summary(&m.parts)),
    ])
}

/// A data part as display lines. Blocked-agent items carry the captured
/// screen as `{"context": "..."}`; show it as text, blank rows dropped.
fn context_lines(d: &serde_json::Value) -> Vec<String> {
    let text = |s: &str| -> Vec<String> {
        s.lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect()
    };
    match d {
        serde_json::Value::Object(m) => m
            .iter()
            .flat_map(|(k, v)| match v.as_str() {
                Some(s) => text(s),
                None => vec![format!("{k}: {v}")],
            })
            .collect(),
        serde_json::Value::String(s) => text(s),
        other => vec![other.to_string()],
    }
}

fn render_detail(f: &mut Frame, area: Rect, m: &Message, ctx: &Ctx) {
    let (exp, exp_style) = expires(m, ctx.now);
    let mut head = vec![Line::from(vec![
        Span::styled(m.id.0.clone(), super::dim()),
        Span::raw(format!(
            "  {} from {} · expires ",
            fmt::wire(m.kind),
            m.from_id
        )),
        Span::styled(exp, exp_style),
    ])];
    let mut context: Vec<Line> = Vec::new();
    for p in &m.parts {
        if let Some(t) = &p.text {
            head.extend(t.lines().map(|l| Line::raw(l.to_string())));
        }
        if let Some(d) = &p.data {
            context.extend(
                context_lines(d)
                    .into_iter()
                    .map(|l| Line::styled(l, super::dim())),
            );
        }
    }
    let hint = if m.kind == MessageKind::Approval {
        "y approve · n deny"
    } else {
        "y yes · n no · r reply"
    };
    let block = Block::bordered().title(format!(" {hint} "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [top, rest] = Layout::vertical([
        Constraint::Length((head.len() as u16).min(inner.height)),
        Constraint::Min(0),
    ])
    .areas(inner);
    f.render_widget(Paragraph::new(head).wrap(Wrap { trim: false }), top);
    // the screen's last rows hold the dialog being asked about
    let skip = context.len().saturating_sub(rest.height as usize);
    f.render_widget(Paragraph::new(context).scroll((skip as u16, 0)), rest);
}
