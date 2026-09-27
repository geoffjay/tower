//! Views (D§11): each owns its selection/filter state, handles its keys
//! (returning `Cmd`s for the app), and renders from the shared `Store`.

pub mod agent;
pub mod events;
pub mod fleet;
pub mod inbox;
pub mod machines;
pub mod tasks;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use tower_core::{AgentId, AgentState, MessageId, TaskId};

use crate::app::{Effect, InputPurpose};
use crate::fmt::Tz;
use crate::store::Store;

/// Read-only render/key context.
pub struct Ctx<'a> {
    pub store: &'a Store,
    pub now: i64,
    pub tz: Tz,
}

/// What a view asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Effect(Effect),
    /// Focus the input line (prefilled).
    Input(InputPurpose, String),
    OpenAgent(AgentId),
    /// Leave a sub-view (agent detail, task detail).
    Back,
    Toast(String),
}

/// Vim-style list navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
}

pub fn nav(k: &KeyEvent) -> Option<Nav> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    Some(match k.code {
        KeyCode::Char('j') | KeyCode::Down if !ctrl => Nav::Down,
        KeyCode::Char('k') | KeyCode::Up if !ctrl => Nav::Up,
        KeyCode::Char('g') | KeyCode::Home => Nav::Top,
        KeyCode::Char('G') | KeyCode::End => Nav::Bottom,
        KeyCode::Char('d') if ctrl => Nav::PageDown,
        KeyCode::Char('u') if ctrl => Nav::PageUp,
        KeyCode::PageDown => Nav::PageDown,
        KeyCode::PageUp => Nav::PageUp,
        _ => return None,
    })
}

const PAGE: usize = 10;

/// Move an id-based selection through `ids` (selection survives re-sorts).
pub fn step<T: PartialEq + Clone>(ids: &[T], sel: &Option<T>, n: Nav) -> Option<T> {
    if ids.is_empty() {
        return None;
    }
    // no (or a vanished) selection renders as row 0; move from there
    let i = index_of(ids, sel).unwrap_or(0);
    let last = ids.len() - 1;
    let idx = match n {
        Nav::Top => 0,
        Nav::Bottom => last,
        Nav::Down => (i + 1).min(last),
        Nav::Up => i.saturating_sub(1),
        Nav::PageDown => (i + PAGE).min(last),
        Nav::PageUp => i.saturating_sub(PAGE),
    };
    Some(ids[idx].clone())
}

/// Index of the selection in `ids`, defaulting to the first row.
pub fn index_of<T: PartialEq>(ids: &[T], sel: &Option<T>) -> Option<usize> {
    if ids.is_empty() {
        return None;
    }
    Some(
        sel.as_ref()
            .and_then(|s| ids.iter().position(|i| i == s))
            .unwrap_or(0),
    )
}

/// The selection, falling back to the first row when unset or gone.
pub fn current<T: PartialEq + Clone>(ids: &[T], sel: &Option<T>) -> Option<T> {
    index_of(ids, sel).map(|i| ids[i].clone())
}

pub fn state_style(s: AgentState) -> Style {
    let c = match s {
        AgentState::Working => Color::Green,
        AgentState::Blocked => Color::Yellow,
        AgentState::Done => Color::Cyan,
        AgentState::Dead => Color::Red,
        AgentState::Launching => Color::Blue,
        _ => Color::Gray,
    };
    Style::new().fg(c)
}

pub fn selected() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}

pub fn header() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

/// Shorthand constructors used by several views.
pub fn respond(msg: &MessageId, approve: Option<bool>, text: Option<String>) -> Cmd {
    Cmd::Effect(Effect::Api(crate::app::Api::Respond {
        msg: msg.clone(),
        approve,
        text,
    }))
}

pub fn cancel_task(task: &TaskId) -> Cmd {
    Cmd::Effect(Effect::Api(crate::app::Api::TaskCancel {
        task: task.clone(),
    }))
}
