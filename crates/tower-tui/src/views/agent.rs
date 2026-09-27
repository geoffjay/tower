//! Agent detail (plan T2.2, D§11/§11.1): live output tail with ANSI
//! passthrough, message history, prompt input (`i`), interrupt (`x`), and
//! `o` to the real pane in herdr.

use ansi_to_tui::IntoText;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use tower_core::{AgentId, Message};

use super::{Cmd, Ctx, Nav};

pub const HINTS: &str = "i prompt · x interrupt · o herdr · j/k scroll · G follow · esc back";
pub const HELP: &[&str] = &[
    "i       prompt this agent (enter sends)",
    "x       interrupt (ctrl+c)",
    "o       show the pane in herdr (focus inside herdr, else attach)",
    "j k     scroll output · ctrl-d/u page · g top · G follow",
    "esc     back to the fleet",
];

#[derive(Debug)]
pub struct AgentView {
    pub id: AgentId,
    /// Lines scrolled up from the bottom (0 = following).
    pub scroll: usize,
}

impl AgentView {
    pub fn new(id: AgentId) -> Self {
        Self { id, scroll: 0 }
    }

    pub fn key(&mut self, k: KeyEvent, ctx: &Ctx) -> Vec<Cmd> {
        if let Some(n) = super::nav(&k) {
            self.scroll = match n {
                Nav::Up => self.scroll + 1,
                Nav::Down => self.scroll.saturating_sub(1),
                Nav::PageUp => self.scroll + 10,
                Nav::PageDown => self.scroll.saturating_sub(10),
                Nav::Top => usize::MAX / 2,
                Nav::Bottom => 0,
            };
            return Vec::new();
        }
        if k.code == KeyCode::Esc {
            return vec![Cmd::Back];
        }
        if k.code == KeyCode::Enter {
            return Vec::new();
        }
        match ctx.store.agent(&self.id) {
            Some(a) => super::fleet::agent_action(k, a),
            None => Vec::new(),
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, ctx: &Ctx) {
        let [head, body] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);
        self.render_head(f, head, ctx);
        let (out_area, hist_area) = if body.width >= 110 {
            let [o, h] =
                Layout::horizontal([Constraint::Percentage(64), Constraint::Percentage(36)])
                    .areas(body);
            (o, h)
        } else {
            let [o, h] = Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)])
                .areas(body);
            (o, h)
        };
        self.render_output(f, out_area, ctx);
        render_history(f, hist_area, ctx, &self.id);
    }

    fn render_head(&self, f: &mut Frame, area: Rect, ctx: &Ctx) {
        let Some(a) = ctx.store.agent(&self.id) else {
            f.render_widget(
                Paragraph::new(Span::styled("agent removed — esc returns", super::dim())),
                area,
            );
            return;
        };
        let st = super::state_style(a.state);
        let mut lines = vec![Line::from(vec![
            Span::styled(format!("{} ", a.state.glyph()), st),
            Span::styled(a.name.clone(), super::header()),
            Span::raw(format!(
                "  {} · {} · ",
                a.kind,
                ctx.store.machine_name(&a.machine_id)
            )),
            Span::styled(a.state.as_str(), st),
        ])];
        let mut where_ = Vec::new();
        if let Some(p) = &a.pane_id {
            where_.push(format!("pane {p}"));
        }
        if let Some(w) = &a.worktree {
            where_.push(format!("worktree {w}"));
        } else if let Some(w) = &a.workdir {
            where_.push(format!("workdir {w}"));
        }
        lines.push(Line::styled(where_.join(" · "), super::dim()));
        let task = match ctx.store.current_task(&a.id) {
            Some(t) => {
                let lease = t
                    .lease_expires_at
                    .map(|e| format!(", lease {}", crate::fmt::countdown(e - ctx.now)))
                    .unwrap_or_default();
                let state = crate::fmt::wire(t.state);
                format!("job  {} ({state}{lease})", t.title)
            }
            None => "job  —".into(),
        };
        lines.push(Line::raw(task));
        f.render_widget(Paragraph::new(lines), area);
    }

    fn render_output(&mut self, f: &mut Frame, area: Rect, ctx: &Ctx) {
        let block = Block::bordered().title(if self.scroll == 0 {
            " output ".to_string()
        } else {
            format!(" output (scrolled ↑{}) ", self.scroll)
        });
        let inner = block.inner(area);
        f.render_widget(block, area);
        let text = match &ctx.store.output {
            Some((id, raw)) if *id == self.id => output_text(raw),
            _ => Text::styled("reading…", super::dim()),
        };
        let total = text.lines.len();
        let h = inner.height as usize;
        let max_up = total.saturating_sub(h);
        self.scroll = self.scroll.min(max_up);
        let top = max_up - self.scroll;
        f.render_widget(Paragraph::new(text).scroll((top as u16, 0)), inner);
    }
}

/// Terminal read → styled text, trailing blank rows dropped so the tail
/// ends at the last real line.
pub fn output_text(raw: &str) -> Text<'static> {
    let mut text = raw
        .into_text()
        .unwrap_or_else(|_| Text::raw(raw.to_string()));
    while text
        .lines
        .last()
        .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
    {
        text.lines.pop();
    }
    text
}

fn render_history(f: &mut Frame, area: Rect, ctx: &Ctx, id: &AgentId) {
    let block = Block::bordered().title(" messages ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let msgs: &[Message] = match &ctx.store.history {
        Some((hid, m)) if hid == id => m,
        _ => &[],
    };
    if msgs.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("no messages", super::dim())),
            inner,
        );
        return;
    }
    let w = inner.width as usize;
    // newest first from the API; show chronologically, newest at the bottom
    let mut lines: Vec<Line> = Vec::new();
    for m in msgs.iter().rev() {
        let (kind, status) = (crate::fmt::wire(m.kind), crate::fmt::wire(m.status));
        lines.push(Line::from(vec![
            Span::styled(ctx.tz.clock(m.created_at, false), super::dim()),
            Span::raw(format!(" {} → {} ", m.from_id, m.to_id)),
            Span::styled(
                format!("{kind} ({status})"),
                Style::new().fg(ratatui::style::Color::Cyan),
            ),
        ]));
        lines.push(Line::raw(format!(
            "  {}",
            crate::fmt::trunc(&crate::fmt::summary(&m.parts), w.saturating_sub(2))
        )));
    }
    let top = lines.len().saturating_sub(inner.height as usize);
    f.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), inner);
}
