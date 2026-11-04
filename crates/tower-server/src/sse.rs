//! The event log over HTTP (D§7): `/v1/events`, the SSE stream, and
//! `/v1/events/history`, the log newest first one page at a time.
//!
//! Stream cursor semantics: client passes `?cursor=<seq>` or
//! `Last-Event-ID`; the server replays everything after the cursor from the
//! event log, then follows live appends with a short poll tick. Heartbeats
//! via SSE keep-alive (15s). Filters (both routes, [`EventFilter`]):
//! `?filter=type:task` keeps types containing the text, `?exclude=type:…`
//! drops them, `?subject=agent:a_1` exact subject match.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::rejection::QueryRejection;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;

use crate::state::AppState;
use crate::storage::EventFilter;

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub cursor: Option<i64>,
    /// `?filter=type:task` — substring match on event type
    pub filter: Option<String>,
    /// `?exclude=type:agent.output` — drop types containing the text
    pub exclude: Option<String>,
    /// `?subject=agent:a_1` — exact `type:id`
    pub subject: Option<String>,
}

/// Extractor combining query params with the SSE `Last-Event-ID` header.
pub struct EventsRequest {
    pub cursor: Option<i64>,
    pub filter: EventFilter,
}

impl<S> FromRequestParts<S> for EventsRequest
where
    S: Send + Sync,
    AppState: axum::extract::FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(q): Query<EventsQuery> =
            Query::from_request_parts(parts, state).await.map_err(|e| {
                let err = tower_core::TowerError::invalid(e.to_string());
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({ "error": err })),
                )
                    .into_response()
            })?;
        let last_event_id = parts
            .headers
            .get("Last-Event-ID")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok());
        Ok(Self {
            cursor: q.cursor.or(last_event_id),
            filter: EventFilter {
                filter: q.filter,
                exclude: q.exclude,
                subject: q.subject,
            },
        })
    }
}

pub async fn events(
    State(state): State<AppState>,
    req: EventsRequest,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let start_cursor = match req.cursor {
        Some(c) => c,
        // no cursor: start live at the head
        None => state.events.latest_seq().await.unwrap_or(0),
    };

    // retention pruning never deletes what this stream has yet to send
    let tracked = state.events.track_cursor(start_cursor);
    let stream = futures::stream::unfold(
        (state, start_cursor, req.filter, tracked),
        |(state, cursor, filter, tracked)| async move {
            loop {
                let batch = state.events.since(cursor, 64).await.unwrap_or_default();
                if batch.is_empty() {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                let mut new_cursor = cursor;
                let mut payload = String::new();
                for e in &batch {
                    if filter.matches(e) {
                        payload.push_str(&serde_json::to_string(e).unwrap_or_default());
                        payload.push('\n');
                    }
                    new_cursor = e.seq;
                }
                tracked.store(new_cursor, std::sync::atomic::Ordering::Relaxed);
                let item = SseEvent::default()
                    .id(new_cursor.to_string())
                    .event("tower-batch")
                    .data(payload.trim_end());
                return Some((Ok(item), (state, new_cursor, filter, tracked)));
            }
        },
    );

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// Default and bounds of a history page's `limit`.
const HISTORY_LIMIT: i64 = 50;
const HISTORY_MAX: i64 = 500;

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub before: Option<i64>,
    pub limit: Option<i64>,
    pub filter: Option<String>,
    pub exclude: Option<String>,
    pub subject: Option<String>,
}

/// `GET /v1/events/history`: one page of the log, newest first, with the
/// head to follow live from (D§7).
pub async fn history(
    State(state): State<AppState>,
    q: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let invalid =
        |m: String| crate::http::error_response(tower_core::TowerError::invalid(m).into());
    let Query(q) = match q {
        Ok(q) => q,
        Err(e) => return invalid(e.body_text()),
    };
    let limit = q.limit.unwrap_or(HISTORY_LIMIT);
    if !(1..=HISTORY_MAX).contains(&limit) {
        return invalid(format!("limit must be 1-{HISTORY_MAX}, got {limit}"));
    }
    let filter = EventFilter {
        filter: q.filter,
        exclude: q.exclude,
        subject: q.subject,
    };
    match state.events.history(q.before, limit, &filter).await {
        Ok(page) => Json(page).into_response(),
        Err(e) => crate::http::error_response(e),
    }
}
