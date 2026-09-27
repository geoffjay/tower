//! Terminal UI (phase 3, D§11): a `/v1` client like any other. tower shows
//! coordination state; herdr shows terminals (`o` jumps there).
//!
//! Runtime: one channel feeds the loop — keys (input thread), SSE batches
//! (feed task), fetch/API results (spawned tasks), the 1s clock, signals.
//! `App` is pure; this module owns the terminal and does the I/O.

pub mod app;
pub mod exec;
pub mod feed;
pub mod fmt;
pub mod input;
pub mod store;
pub mod views;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event as TermEvent, KeyEvent, KeyEventKind};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tower_client::Client;

use crate::app::{App, Effect};
use crate::feed::FeedMsg;
use crate::store::{Fetch, Loaded};

enum Msg {
    Key(KeyEvent),
    Redraw,
    Feed(FeedMsg),
    Loaded(Fetch, Result<Vec<Loaded>, String>),
    Api(Result<String, String>),
    Tick,
    Quit,
}

impl From<FeedMsg> for Msg {
    fn from(m: FeedMsg) -> Self {
        Msg::Feed(m)
    }
}

/// Run the TUI until the operator quits (or the terminal hangs up).
pub async fn run(client: Client) -> anyhow::Result<()> {
    let (tx, mut rx) = unbounded_channel::<Msg>();
    let paused = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    spawn_input(tx.clone(), paused.clone(), stop.clone());
    let background = [
        tokio::spawn(feed::run(client.clone(), tx.clone())),
        tokio::spawn(ticker(tx.clone())),
        tokio::spawn(signals(tx.clone())),
    ];

    let mut terminal = ratatui::try_init()?;
    let mut app = App::new(tower_core::now_ms(), fmt::Tz::Local);
    let res = event_loop(&mut terminal, &mut app, &client, &tx, &mut rx, &paused).await;

    stop.store(true, Ordering::Relaxed);
    for task in background {
        task.abort();
    }
    ratatui::restore();
    res
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    client: &Client,
    tx: &UnboundedSender<Msg>,
    rx: &mut UnboundedReceiver<Msg>,
    paused: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| app.render(f))?;
        let Some(first) = rx.recv().await else {
            return Ok(());
        };
        // coalesce bursts (event replays, key repeat) into one redraw
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        for m in batch {
            // countdowns and toast lifetimes use the real clock, not the
            // last tick (an attach blocks the loop for as long as it lasts)
            app.now = tower_core::now_ms();
            let effects = match m {
                Msg::Key(k) => app.on_key(k),
                Msg::Redraw => Vec::new(),
                Msg::Feed(f) => app.on_feed(f),
                Msg::Loaded(f, r) => app.on_loaded(f, r),
                Msg::Api(r) => {
                    app.on_api(r);
                    Vec::new()
                }
                Msg::Tick => app.tick(tower_core::now_ms()),
                Msg::Quit => {
                    app.quit = true;
                    Vec::new()
                }
            };
            for e in effects {
                match e {
                    Effect::Fetch(f) => {
                        let (c, tx) = (client.clone(), tx.clone());
                        tokio::spawn(async move {
                            let r = exec::fetch(&c, &f).await.map_err(|e| format!("{e:#}"));
                            let _ = tx.send(Msg::Loaded(f, r));
                        });
                    }
                    Effect::Api(a) => {
                        app.on_api_started(&a);
                        let (c, tx) = (client.clone(), tx.clone());
                        tokio::spawn(async move {
                            let r = exec::api(&c, &a).await.map_err(|e| format!("{e:#}"));
                            let _ = tx.send(Msg::Api(r));
                        });
                    }
                    Effect::Attach { target } => {
                        let r = attach(terminal, paused, &target).await;
                        app.now = tower_core::now_ms();
                        app.on_api(r);
                    }
                }
            }
        }
        if app.quit {
            return Ok(());
        }
    }
}

/// `o`: inside herdr, focus the agent's pane (the TUI keeps running in its
/// own pane); outside, hand the terminal to `herdr agent attach` and take
/// it back when that exits (D§11 division of labor).
async fn attach(
    terminal: &mut ratatui::DefaultTerminal,
    paused: &Arc<AtomicBool>,
    target: &str,
) -> Result<String, String> {
    let target = target.to_string();
    if std::env::var_os("HERDR_PANE_ID").is_some() {
        let t = target.clone();
        let out = tokio::task::spawn_blocking(move || {
            std::process::Command::new("herdr")
                .args(["agent", "focus", &t])
                .output()
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("herdr: {e}"))?;
        return if out.status.success() {
            Ok(format!("focused {target} in herdr"))
        } else {
            Err(format!(
                "herdr agent focus {target}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        };
    }
    // stop reading keys so herdr gets them, then give it the terminal
    paused.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(150)).await;
    ratatui::restore();
    let t = target.clone();
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("herdr")
            .args(["agent", "attach", &t])
            .status()
    })
    .await;
    *terminal = ratatui::init();
    let _ = terminal.clear();
    paused.store(false, Ordering::Relaxed);
    match status {
        Ok(Ok(s)) if s.success() => Ok(format!("back from {target}")),
        Ok(Ok(s)) => Err(format!("herdr agent attach {target} exited {s}")),
        Ok(Err(e)) => Err(format!("herdr: {e}")),
        Err(e) => Err(e.to_string()),
    }
}

/// Terminal input on a plain thread (crossterm's reader blocks). Pausable
/// so a child process (herdr attach) owns stdin while it runs.
fn spawn_input(tx: UnboundedSender<Msg>, paused: Arc<AtomicBool>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) && !tx.is_closed() {
            if paused.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            let msg = match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(false) => continue,
                Ok(true) => match crossterm::event::read() {
                    Ok(TermEvent::Key(k)) if k.kind != KeyEventKind::Release => Msg::Key(k),
                    Ok(TermEvent::Resize(..)) => Msg::Redraw,
                    Ok(_) => continue,
                    Err(_) => Msg::Quit,
                },
                // the terminal went away (hangup): shut down cleanly
                Err(_) => Msg::Quit,
            };
            let quit = matches!(msg, Msg::Quit);
            if tx.send(msg).is_err() || quit {
                return;
            }
        }
    });
}

async fn ticker(tx: UnboundedSender<Msg>) {
    let mut every = tokio::time::interval(Duration::from_secs(1));
    loop {
        every.tick().await;
        if tx.send(Msg::Tick).is_err() {
            return;
        }
    }
}

/// SIGHUP / SIGTERM → quit through the normal path (terminal restored).
async fn signals(tx: UnboundedSender<Msg>) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut hup), Ok(mut term)) = (
        signal(SignalKind::hangup()),
        signal(SignalKind::terminate()),
    ) else {
        return;
    };
    tokio::select! {
        _ = hup.recv() => {}
        _ = term.recv() => {}
    }
    let _ = tx.send(Msg::Quit);
}
