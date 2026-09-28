//! Settings page and command palette render checks (web QOL: theme
//! selection). The palette's keyboard flow and theme persistence are
//! browser behavior — verified live in Chrome, not here; these tests pin
//! the served markup: every page embeds the theme variables, the palette
//! dialog with the registered commands, and the settings page renders
//! the dropdown with every registered theme.

use std::sync::Arc;

use tower_core::{Event, Machine};
use tower_web::{Snapshot, UiSource};

/// Smallest source the UI accepts: an empty roster, no events.
struct Fixture {
    head: tokio::sync::watch::Sender<i64>,
}

impl Fixture {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            head: tokio::sync::watch::channel(0).0,
        })
    }
}

#[async_trait::async_trait]
impl UiSource for Fixture {
    async fn snapshot(&self) -> anyhow::Result<Snapshot> {
        Ok(Snapshot {
            machines: Vec::<Machine>::new(),
            agents: vec![],
            open_tasks: vec![],
            pending_from_agents: vec![],
        })
    }
    async fn events_since(&self, _cursor: i64, _limit: i64) -> anyhow::Result<Vec<Event>> {
        Ok(vec![])
    }
    async fn cursor_at(&self, _ts: i64) -> anyhow::Result<i64> {
        Ok(0)
    }
    fn head(&self) -> tokio::sync::watch::Receiver<i64> {
        self.head.subscribe()
    }
}

async fn get(app: &axum::Router, uri: &str) -> (axum::http::StatusCode, String) {
    let req = axum::http::Request::get(uri)
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = tower::ServiceExt::oneshot(app.clone(), req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

const IDS: &str = "[\"tower-dark\", \"tokyo-night-storm\", \"tokyo-night-light\"]";

#[tokio::test]
async fn every_page_carries_themes_palette_and_boot_script() {
    let app = tower_web::router(Fixture::new());
    for uri in ["/ui", "/ui/settings"] {
        let (status, html) = get(&app, uri).await;
        assert_eq!(status, 200, "{uri}");
        // theme variables for every registered theme + the light fixes
        assert!(html.contains("[data-theme=\"tower-dark\"]"), "{uri}");
        assert!(html.contains("[data-theme=\"tokyo-night-storm\"]"), "{uri}");
        assert!(html.contains("[data-theme=\"tokyo-night-light\"]"), "{uri}");
        // the stylesheet is served verbatim: a child combinator must not
        // arrive as `&gt;` (a <style> element does not decode entities)
        assert!(html.contains(".palette > div"), "{uri} css escaped");
        // the boot script validates against the registry ids and falls
        // back to the default on junk/missing storage
        assert!(
            html.contains(&format!("if (!{IDS}.includes(t)) t = \"tower-dark\";")),
            "{uri} boot"
        );
        // the palette dialog and both commands
        assert!(
            html.contains("<dialog class=\"palette\" id=\"palette\""),
            "{uri}"
        );
        assert!(html.contains("data-palette-open"), "{uri} header button");
        assert!(html.contains("Search commands…"), "{uri}");
        assert!(
            html.contains("[\"Agent cloud\", \"fleet health at a glance\", \"/ui\"]"),
            "{uri} cloud command"
        );
        assert!(
            html.contains("[\"Settings\", \"theme and UI options\", \"/ui/settings\"]"),
            "{uri} settings command"
        );
        // ⌘K handler
        assert!(
            html.contains("e.key === 'k' && (e.metaKey || e.ctrlKey)"),
            "{uri}"
        );
    }
}

#[tokio::test]
async fn settings_page_renders_theme_dropdown() {
    let app = tower_web::router(Fixture::new());
    let (status, html) = get(&app, "/ui/settings").await;
    assert_eq!(status, 200);
    assert!(html.contains("tower · settings"));
    assert!(html.contains("<select id=\"theme-select\">"));
    assert!(html.contains("<option value=\"tower-dark\">Tower Dark (default)</option>"));
    assert!(html.contains("<option value=\"tokyo-night-storm\">Tokyo Night Storm</option>"));
    assert!(html.contains("<option value=\"tokyo-night-light\">Tokyo Night Light</option>"));
    // the save script validates against the registry ids
    assert!(html.contains(&format!("const ids = {IDS};")), "save script");
    assert!(html.contains("localStorage.setItem('tower.theme'"));
}

#[tokio::test]
async fn settings_page_is_its_own_page_not_a_redirect() {
    let app = tower_web::router(Fixture::new());
    let (status, html) = get(&app, "/ui/settings").await;
    assert_eq!(status, 200);
    assert!(html.contains("<h1>Settings</h1>"));
}

#[tokio::test]
async fn cloud_page_still_renders() {
    let app = tower_web::router(Fixture::new());
    let (status, html) = get(&app, "/ui").await;
    assert_eq!(status, 200);
    assert!(html.contains("tower · agent cloud"));
    assert!(html.contains("no agents yet"));
}
