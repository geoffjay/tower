//! Server side of the web UI (D§12, D§13): the data source `tower-web`
//! renders from, the login link that sets the UI cookie, and the route
//! `tower ui` asks for the link's token.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use tower_core::{Event, MessageStatus, PartyKind};

use crate::auth::{self, UI_COOKIE, UI_LOGIN_PATH};
use crate::state::AppState;

/// Browser keeps the UI login this long (seconds).
const COOKIE_MAX_AGE_S: u32 = 30 * 24 * 3600;

/// `/`, `/ui/login`, `/v1/ui/token`, and the `tower-web` routes.
pub fn router(state: AppState) -> Router {
    let web = tower_web::router(Arc::new(Source(state.clone())));
    Router::new()
        .route("/", get(|| async { Redirect::to("/ui") }))
        .route(UI_LOGIN_PATH, get(login))
        .route("/v1/ui/token", get(token))
        .with_state(state)
        .merge(web)
}

struct Source(AppState);

#[async_trait::async_trait]
impl tower_web::UiSource for Source {
    async fn snapshot(&self) -> anyhow::Result<tower_web::Snapshot> {
        let s = &self.0;
        let pending = crate::messaging::list(
            s,
            &crate::messaging::ListQuery {
                status: Some(MessageStatus::Pending),
                ..Default::default()
            },
        )
        .await?;
        Ok(tower_web::Snapshot {
            machines: crate::inventory::list_machines(s).await?,
            agents: crate::inventory::list_agents(s).await?,
            open_tasks: crate::tasks::open_jobs(s).await?,
            pending_from_agents: pending
                .into_iter()
                .filter(|m| m.from_kind == PartyKind::Agent)
                .collect(),
        })
    }

    async fn events_since(&self, cursor: i64, limit: i64) -> anyhow::Result<Vec<Event>> {
        self.0.events.since(cursor, limit).await
    }

    async fn cursor_at(&self, ts: i64) -> anyhow::Result<i64> {
        self.0.events.cursor_at(ts).await
    }

    fn head(&self) -> tokio::sync::watch::Receiver<i64> {
        self.0.events.subscribe()
    }
}

#[derive(Deserialize)]
struct LoginQuery {
    token: Option<String>,
}

/// `GET /ui/login?token=` — valid UI token: set the cookie and send the
/// browser to `/ui` (the token leaves the address bar).
async fn login(State(state): State<AppState>, Query(q): Query<LoginQuery>) -> Response {
    let want = auth::ui_token(&state.token);
    if !q.token.is_some_and(|t| auth::constant_time_eq(&t, &want)) {
        return auth::ui_login_needed();
    }
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, "/ui".to_string()),
            (
                header::SET_COOKIE,
                format!(
                    "{UI_COOKIE}={want}; HttpOnly; SameSite=Strict; Path=/ui; Max-Age={COOKIE_MAX_AGE_S}"
                ),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
    )
        .into_response()
}

/// `GET /v1/ui/token` (bearer) — what `tower ui` turns into a login link.
async fn token(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "token": auth::ui_token(&state.token),
        "login_path": UI_LOGIN_PATH,
    }))
}
