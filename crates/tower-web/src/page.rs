//! The agent cloud page (D§12.1–12.2): one connected page with two live
//! regions — the cloud + widgets, and the detail panel for the selected
//! agent. Each region re-renders on the server when the metrics follower
//! applies new events (coalesced) or its refresh tick fires, and the
//! runtime morphs the result in place over Topcoat's WebSocket.

use std::sync::Arc;
use std::time::Duration;

use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{Event, Signal, connected, signal};
use topcoat::view::*;

use crate::Ui;
use crate::metrics::BUCKETS;
use crate::model::{Attention, Cloud, Panel, Point};

const CSS: &str = include_str!("style.css");
/// Windows drain and ages grow without events: re-render at least this often.
const CLOUD_REFRESH: Duration = Duration::from_secs(5);
/// The panel's lease countdown ticks.
const PANEL_TICK: Duration = Duration::from_secs(1);

/// Legend order = the D§12.1 color table.
const STATES: [&str; 7] = [
    "working",
    "blocked",
    "idle",
    "done",
    "dead",
    "launching",
    "unknown",
];

#[page("/ui")]
pub async fn cloud_page(cx: &Cx) -> Result<impl View> {
    // the selected agent's id; "" = none. Browser-held: validated by lookup.
    let selected = signal(cx, String::new);
    let for_cloud = selected.clone();
    let for_panel = selected.clone();
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>"tower · agent cloud"</title>
                topcoat::runtime::script()
                <style>(CSS)</style>
            </head>
            <body @keydown=$(|e: Event| if e.key == "Escape" { selected.set("".to_owned()) } else {})>
                (live! {
                    let ui = app_context::<Arc<Ui>>(cx);
                    let mut applied = ui.applied.subscribe();
                    loop {
                        applied.borrow_and_update();
                        let token = match ui.cloud().await {
                            Ok(model) => emit! { cloud_view(model: model, selected: for_cloud.clone()) }?,
                            Err(e) => emit! { <div class="offline">"tower data unavailable: " (e.to_string())</div> }?,
                        };
                        if !connected(cx) || !ui.wait(&mut applied, CLOUD_REFRESH).await {
                            break Ok(token);
                        }
                    }
                })
                (live! {
                    let ui = app_context::<Arc<Ui>>(cx);
                    let mut applied = ui.applied.subscribe();
                    // tracked read: a new selection re-renders the page over the socket
                    let agent = for_panel.get();
                    loop {
                        applied.borrow_and_update();
                        let model = if agent.is_empty() {
                            None
                        } else {
                            ui.panel(&agent).await.unwrap_or(None)
                        };
                        let token = emit! { panel_view(model: model, selected: for_panel.clone()) }?;
                        if agent.is_empty() || !connected(cx) || !ui.wait(&mut applied, PANEL_TICK).await {
                            break Ok(token);
                        }
                    }
                })
            </body>
        </html>
    })
}

fn point_class(p: &Point) -> String {
    let halo = match p.attention {
        Attention::Needs => " needs",
        Attention::Fault => " fault",
        Attention::None => "",
    };
    format!("pt {}{halo}", p.state.as_str())
}

fn tooltip(p: &Point) -> String {
    let halo = match p.attention {
        Attention::Needs => "\nneeds you (blocked or waiting on an answer)",
        Attention::Fault => "\nrecent fault (lease lost, job failed, approval expired)",
        Attention::None => "",
    };
    format!(
        "{} · {}\nactivity: {} events in the last 5 min (size)\nhealth: {:.0}% — lease, faults, silence (brightness){halo}",
        p.name,
        p.state.as_str(),
        p.activity,
        p.health * 100.0
    )
}

#[component]
async fn cloud_view(model: Cloud, selected: Signal<String>) -> Result<impl View> {
    let empty = model.points.is_empty();
    let queue = model.queue;
    Ok(view! {
        <header class="bar">
            <span class="brand">"tower"</span>
            <span class="pool" title="open jobs — blocked: input-required, or its owner is blocked">
                <b>(queue.queued)</b>" queued · "
                <b>(queue.working)</b>" working · "
                <b class="warn">(queue.blocked)</b>" blocked"
            </span>
            if model.needs_you > 0 {
                <span class="needs-badge" title="blocked agents and agents waiting on an answer; answer in the TUI or `tower inbox`">
                    (model.needs_you)" need you · "(model.inbox)" in inbox"
                </span>
            }
            <span class="machines">
                for m in model.machines {
                    <span class=(if m.online { "chip on" } else { "chip off" }) title=(format!("{} agents", m.agents))>
                        <i></i>(m.name)
                    </span>
                }
            </span>
            <span class="legend">
                for s in STATES {
                    <span><i class=(format!("dot {s}"))></i>(s)</span>
                }
            </span>
        </header>
        <svg id="cloud" viewBox=(model.view_box) preserveAspectRatio="xMidYMid meet">
            <rect class="bg" x="-5000" y="-5000" width="10000" height="10000" @click=$(|_e| selected.set("".to_owned()))></rect>
            for c in model.clusters {
                <text class="cluster" x=(format!("{:.0}", c.x)) y=(format!("{:.0}", c.y))>(c.name)</text>
            }
            #[key(p.id.clone())]
            for p in model.points {
                let pid = p.id.clone();
                <g
                    id=(format!("a-{}", p.id))
                    class=(point_class(&p))
                    style=(format!("transform: translate({:.1}px, {:.1}px)", p.x, p.y))
                    :data-selected=$(selected.get() == pid)
                    @click=$(|_e| selected.set(pid.to_owned()))
                >
                    <title>(tooltip(&p))</title>
                    <g class="drift" style=(format!("animation-duration: {}s; animation-delay: -{}s", p.drift_s, p.drift_phase_s))>
                        <circle class="halo" style=(format!("r: {:.1}px", p.r + 7.0))></circle>
                        <circle class="core" style=(format!("r: {:.1}px; fill-opacity: {:.2}", p.r, p.health))></circle>
                        <text class="label" y=(format!("{:.1}", p.r + 16.0))>(p.name)</text>
                    </g>
                </g>
            }
            if empty {
                <text class="empty" x="500" y="320">"no agents yet — tower spawn NAME --kind K"</text>
            }
        </svg>
        <footer class="ribbon">
            #[key(r.seq)]
            for (i, r) in model.ribbon.into_iter().enumerate() {
                <span class="ev" style=(format!("opacity: {:.2}", 1.0 - i as f64 * 0.08))>
                    <b>(r.age)</b>" "(r.kind)" "<em>(r.who)</em>
                </span>
            }
        </footer>
    })
}

fn lease_text(left: Option<i64>) -> String {
    match left {
        None => "lease held".to_string(),
        Some(0) => "lease expired".to_string(),
        Some(s) => format!("lease {}:{:02}", s / 60, s % 60),
    }
}

fn spark_bars(series: &[u32; BUCKETS]) -> Vec<(f64, f64)> {
    let max = series.iter().copied().max().unwrap_or(0).max(1);
    series
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let h = 28.0 * f64::from(v) / f64::from(max);
            (i as f64 * 5.0, h)
        })
        .collect()
}

#[component]
async fn panel_view(model: Option<Panel>, selected: Signal<String>) -> Result<impl View> {
    Ok(view! {
        match model {
            None => <aside id="panel" class="panel closed"></aside>,
            Some(p) => {
                <aside id="panel" class=(format!("panel {}", p.state.as_str()))>
                    <button class="close" title="close (Esc)" @click=$(|_e| selected.set("".to_owned()))>"×"</button>
                    <h2><i class=(format!("dot {}", p.state.as_str()))></i>(p.name.clone())</h2>
                    match p.attention {
                        Attention::Needs => <p class="attn attn-needs">"needs you — answer in the TUI or `tower inbox`"</p>,
                        Attention::Fault => <p class="attn attn-fault">"recent fault in the last 15 min"</p>,
                        Attention::None => "",
                    }
                    <dl>
                        <dt>"kind"</dt><dd>(p.kind.clone())</dd>
                        <dt>"machine"</dt><dd>(p.machine.clone())</dd>
                        <dt>"state"</dt><dd>(p.state.as_str())</dd>
                        <dt>"job"</dt>
                        <dd>
                            match p.job.clone() {
                                None => "—",
                                Some(j) => {
                                    (j.title)" · "(j.state)
                                    <span class="lease">" · "(lease_text(j.lease_left_s))</span>
                                },
                            }
                        </dd>
                        <dt title="messages to or from it per minute, last 5 min">"messages"</dt>
                        <dd>(format!("{:.1}/min", p.msgs_per_min))</dd>
                        <dt title="events in the last 5 min (point size)">"activity"</dt>
                        <dd>(p.activity)</dd>
                        <dt title="lease, faults, silence (point brightness)">"health"</dt>
                        <dd>(format!("{:.0}%", p.health * 100.0))</dd>
                    </dl>
                    <svg class="spark" viewBox="0 0 150 30" preserveAspectRatio="none">
                        <title>"events per 10 s, last 5 min"</title>
                        for (x, h) in spark_bars(&p.series) {
                            <rect x=(format!("{x:.0}")) y=(format!("{:.1}", 30.0 - h)) width="4" height=(format!("{h:.1}"))></rect>
                        }
                    </svg>
                    match p.snippet.clone() {
                        Some(s) => <pre class="snippet">(s)</pre>,
                        None => <p class="quiet">"no recent output"</p>,
                    }
                </aside>
            },
        }
    })
}
