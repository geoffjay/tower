//! Machines (plan T4.2, D§11): inventory + node status. Phases 1-4 only
//! have `local`; node rows (phase 5) render from the same shape.

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use tower_core::{Machine, MachineId};

use super::{Cmd, Ctx};
use crate::fmt;

pub const HINTS: &str = "j/k move";
pub const HELP: &[&str] = &["j k     move"];

#[derive(Debug, Default)]
pub struct Machines {
    pub sel: Option<MachineId>,
    table: TableState,
}

fn status_style(status: &str) -> Style {
    match status {
        "online" | "connected" => Style::new().fg(Color::Green),
        "offline" | "disconnected" => Style::new().fg(Color::Red),
        _ => Style::new().fg(Color::Yellow),
    }
}

impl Machines {
    pub fn key(&mut self, k: KeyEvent, ctx: &Ctx) -> Vec<Cmd> {
        if let Some(n) = super::nav(&k) {
            let ids: Vec<MachineId> = ctx.store.machines.iter().map(|m| m.id.clone()).collect();
            self.sel = super::step(&ids, &self.sel, n);
        }
        Vec::new()
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, ctx: &Ctx) {
        let ms = &ctx.store.machines;
        if ms.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled("no machines loaded", super::dim())),
                area,
            );
            return;
        }
        let ids: Vec<&MachineId> = ms.iter().map(|m| &m.id).collect();
        self.table.select(super::index_of(&ids, &self.sel.as_ref()));
        let rows: Vec<Row> = ms.iter().map(|m| row(m, ctx)).collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(14),
                Constraint::Length(12),
                Constraint::Length(9),
                Constraint::Length(22),
                Constraint::Length(10),
                Constraint::Min(6),
            ],
        )
        .header(
            Row::new(["NAME", "ROLE", "STATUS", "ADDRESS", "LAST SEEN", "AGENTS"])
                .style(super::header()),
        )
        .row_highlight_style(super::selected());
        f.render_stateful_widget(table, area, &mut self.table);
    }
}

fn row<'a>(m: &'a Machine, ctx: &Ctx) -> Row<'a> {
    let agents = ctx
        .store
        .agents
        .iter()
        .filter(|a| a.machine_id == m.id)
        .count();
    Row::new(vec![
        Cell::from(m.name.as_str()),
        Cell::from(m.role.as_str()),
        Cell::from(Span::styled(m.status.as_str(), status_style(&m.status))),
        Cell::from(m.address.as_deref().unwrap_or("—")),
        Cell::from(
            m.last_seen_at
                .map_or("—".into(), |t| format!("{} ago", fmt::age(ctx.now - t))),
        ),
        Cell::from(agents.to_string()),
    ])
}
