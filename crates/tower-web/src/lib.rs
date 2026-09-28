//! Web UI: the read-only agent cloud (phase 4, D§12), built with Topcoat.
//!
//! Isolation rule (D§12.3): Topcoat types stay inside this crate. The
//! server hands in a [`UiSource`] and mounts the plain `axum::Router`
//! [`router`] returns; auth stays in the server.

mod assets;
mod layout;
mod metrics;
mod model;
mod pages;
mod shell;
mod source;
mod theme;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::routing::{any, get};

pub use source::{Snapshot, UiSource};

/// Events per follower read.
const BATCH: i64 = 512;
/// Bursts of events render once: wait this long after a change.
const COALESCE: Duration = Duration::from_millis(150);
const RETRY: Duration = Duration::from_secs(1);
const PRUNE_EVERY_MS: i64 = 60_000;

/// Shared by every render: the data source, the metrics cache, and how far
/// the cache has applied the event log.
pub(crate) struct Ui {
    source: Arc<dyn UiSource>,
    metrics: parking_lot::Mutex<metrics::Metrics>,
    applied: tokio::sync::watch::Sender<i64>,
}

impl Ui {
    async fn cloud(&self) -> anyhow::Result<model::Cloud> {
        let snap = self.source.snapshot().await?;
        let now = self.source.now();
        Ok(model::cloud(&snap, &self.metrics.lock(), now))
    }

    async fn panel(&self, agent_id: &str) -> anyhow::Result<Option<model::Panel>> {
        let snap = self.source.snapshot().await?;
        let now = self.source.now();
        Ok(model::panel(&snap, &self.metrics.lock(), agent_id, now))
    }

    /// Wait for newly applied events (then let the burst settle) or `max`.
    /// `false` once the follower is gone.
    async fn wait(&self, applied: &mut tokio::sync::watch::Receiver<i64>, max: Duration) -> bool {
        tokio::select! {
            changed = applied.changed() => {
                if changed.is_err() {
                    return false;
                }
                tokio::time::sleep(COALESCE).await;
                true
            }
            () = tokio::time::sleep(max) => true,
        }
    }
}

/// Follow the event log into the metrics cache. Boots from the last fault
/// window only, so retention never deletes a row it needs. Ends when the
/// source's head channel closes.
async fn follow(ui: Arc<Ui>) {
    let mut head = ui.source.head();
    let start = ui.source.now() - metrics::FAULT_WINDOW_MS;
    let mut cursor = loop {
        match ui.source.cursor_at(start).await {
            Ok(c) => break c,
            Err(e) => {
                tracing::warn!(error = %e, "web: event cursor unavailable; retrying");
                tokio::time::sleep(RETRY).await;
            }
        }
    };
    let mut last_prune = ui.source.now();
    loop {
        head.borrow_and_update();
        loop {
            match ui.source.events_since(cursor, BATCH).await {
                Ok(batch) if batch.is_empty() => break,
                Ok(batch) => {
                    let mut m = ui.metrics.lock();
                    for e in &batch {
                        m.apply(e);
                    }
                    cursor = batch.last().map_or(cursor, |e| e.seq);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "web: event read failed; retrying");
                    tokio::time::sleep(RETRY).await;
                }
            }
        }
        let now = ui.source.now();
        if now - last_prune >= PRUNE_EVERY_MS {
            ui.metrics.lock().prune(now);
            last_prune = now;
        }
        ui.applied.send_replace(cursor);
        if head.changed().await.is_err() {
            return;
        }
    }
}

/// The `/ui` routes. Starts the metrics follower, so call it inside a
/// tokio runtime. Mount behind the server's auth layer.
pub fn router(source: Arc<dyn UiSource>) -> axum::Router {
    use topcoat::asset::RouterBuilderAssetExt;
    use topcoat::runtime::RouterBuilderRuntimeExt;

    let ui = Arc::new(Ui {
        source,
        metrics: parking_lot::Mutex::new(metrics::Metrics::default()),
        applied: tokio::sync::watch::channel(0).0,
    });
    tokio::spawn(follow(ui.clone()));

    let app = topcoat::router::Router::builder()
        .page(pages::cloud_page)
        .page(pages::settings_page)
        .assets(assets::config())
        .app_context(ui)
        .runtime()
        .build();

    axum::Router::new()
        .route(
            &format!("{}/{}", assets::PREFIX, assets::SCRIPT_FILE),
            get(assets::runtime_script),
        )
        .route("/ui", any(bridge))
        .route("/ui/{*rest}", any(bridge))
        .with_state(Arc::new(app))
}

/// Hand an axum request to the Topcoat router. Extensions pass through,
/// including hyper's upgrade handle for the runtime's WebSocket.
async fn bridge(
    State(app): State<Arc<topcoat::router::Router>>,
    req: Request,
) -> axum::response::Response {
    app.handle(req.map(topcoat::router::Body::new))
        .await
        .map(axum::body::Body::new)
}
