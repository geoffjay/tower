//! Inbox (plan T3.1, D§11): pending questions/approvals addressed to me,
//! with deadlines; reply inline — `y`/`n`, or free text for questions.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use tower_core::{Message, MessageId, MessageKind};

use super::{Cmd, Ctx};
use crate::app::InputPurpose;
use crate::fmt;

pub const HINTS: &str = "y approve/yes · n deny/no · r reply · d dismiss";
pub const HELP: &[&str] = &[
    "y       approve (approval) or answer \"yes\" (question)",
    "n       deny (approval) or answer \"no\" (question)",
    "r/enter answer a question with free text",
    "d       dismiss — stale question, do nothing with it",
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
            KeyCode::Char('d') => vec![super::dismiss(&m.id)],
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
    let mut context: Vec<String> = Vec::new();
    for p in &m.parts {
        if let Some(t) = &p.text {
            head.extend(t.lines().map(|l| Line::raw(l.to_string())));
        }
        if let Some(d) = &p.data {
            context.extend(context_lines(d));
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
    // pre-wrap to the pane width so the row counts below are rows, not
    // source lines — a long text part must not overflow its allocation
    let width = inner.width as usize;
    let head: Vec<Line> = head
        .iter()
        .flat_map(|l| {
            let s = plain(l);
            wrap(&s, width).into_iter().map(Line::raw)
        })
        .collect();
    let [top, rest] = Layout::vertical([
        Constraint::Length((head.len() as u16).min(inner.height)),
        Constraint::Min(0),
    ])
    .areas(inner);
    f.render_widget(Paragraph::new(head), top);
    // long captured-screen lines wrap instead of clipping at the pane edge
    let context: Vec<Line> = context
        .iter()
        .flat_map(|l| wrap(l, rest.width as usize))
        .map(|l| Line::styled(l, super::dim()))
        .collect();
    // the screen's last rows hold the dialog being asked about
    let skip = context.len().saturating_sub(rest.height as usize);
    f.render_widget(Paragraph::new(context).scroll((skip as u16, 0)), rest);
}

/// The rendered text of a line, spans joined.
fn plain(l: &Line) -> String {
    l.spans.iter().map(|s| s.content.to_string()).collect()
}

/// Greedy character wrap, preserving leading indent. Zero-width panes get
/// the text unchanged.
fn wrap(s: &str, width: usize) -> Vec<String> {
    if width == 0 || s.chars().count() <= width {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let indent_len = s.len() - s.trim_start().len();
    let (indent, body) = s.split_at(indent_len);
    let mut line = String::from(indent);
    let mut cols = indent.chars().count();
    for word in body.split(' ') {
        let w = word.chars().count();
        // an over-long word is hard-split rather than overflowing the pane
        if w > width.saturating_sub(indent.chars().count()) {
            for ch in word.chars() {
                if cols >= width {
                    out.push(std::mem::take(&mut line));
                    line = String::from(indent);
                    cols = indent.chars().count();
                }
                line.push(ch);
                cols += 1;
            }
            continue;
        }
        let add = w + usize::from(!line.is_empty());
        if cols + add > width {
            out.push(std::mem::take(&mut line));
            line = String::from(indent);
            cols = indent.chars().count();
        }
        if !line.ends_with(char::is_whitespace) && !line.is_empty() {
            line.push(' ');
            cols += 1;
        }
        line.push_str(word);
        cols += w;
    }
    out.push(line);
    out
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn short_lines_pass_through() {
        assert_eq!(wrap("hello", 10), vec!["hello"]);
        assert_eq!(wrap("hello", 0), vec!["hello"]);
    }

    #[test]
    fn wraps_at_width() {
        let out = wrap("aaa bbb ccc ddd", 7);
        assert!(out.iter().all(|l| l.chars().count() <= 7));
        assert_eq!(out, vec!["aaa bbb", "ccc ddd"]);
    }

    #[test]
    fn preserves_indent() {
        let out = wrap("  aa bb cc", 5);
        assert_eq!(out, vec!["  aa", "  bb", "  cc"]);
    }

    #[test]
    fn hard_splits_overlong_words() {
        let out = wrap("abcdefghij", 4);
        assert_eq!(out, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn narrow_width_preserves_chars_and_bounds() {
        let s = "the quick brown fox jumps over the lazy dog";
        for width in 1..12 {
            let out = wrap(s, width);
            assert!(out.iter().all(|l| l.chars().count() <= width));
            let got: String = out.concat().chars().filter(|c| *c != ' ').collect();
            let want: String = s.chars().filter(|c| *c != ' ').collect();
            assert_eq!(got, want, "width {width}");
        }
    }
}
