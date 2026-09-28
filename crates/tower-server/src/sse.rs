//! SSE event stream: /v1/events (D§7 streaming).
//!
//! Cursor semantics: client passes `?cursor=<seq>` or `Last-Event-ID`; the
//! server replays everything after the cursor from the event log, then
//! follows live appends with a short poll tick. Heartbeats via SSE
//! keep-alive (15s). Filters: `?filter=type:task` substring on event type,
//! `?subject=agent:a_1` exact subject match.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream::Stream;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub cursor: Option<i64>,
    /// `?filter=type:task` — substring match on event type
    pub filter: Option<String>,
    /// `?subject=agent:a_1` — exact `type:id`
    pub subject: Option<String>,
}

/// Extractor combining query params with the SSE `Last-Event-ID` header.
pub struct EventsRequest {
    pub cursor: Option<i64>,
    pub filter: Option<String>,
    pub subject: Option<String>,
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
            filter: q.filter,
            subject: q.subject,
        })
    }
}

fn matches(e: &tower_core::Event, filter: &Option<String>, subject: &Option<String>) -> bool {
    if let Some(f) = filter {
        let needle = f.strip_prefix("type:").unwrap_or(f);
        if !e.kind.as_str().contains(needle) {
            return false;
        }
    }
    if let Some(want) = subject {
        let got = e
            .subject_type
            .as_deref()
            .zip(e.subject_id.as_deref())
            .map(|(t, id)| format!("{t}:{id}"))
            .unwrap_or_default();
        if !got.eq_ignore_ascii_case(want) {
            return false;
        }
    }
    true
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
        (state, start_cursor, req.filter, req.subject, tracked),
        |(state, cursor, filter, subject, tracked)| async move {
            loop {
                let batch = state.events.since(cursor, 64).await.unwrap_or_default();
                if batch.is_empty() {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                let mut new_cursor = cursor;
                let mut payload = String::new();
                for e in &batch {
                    if matches(e, &filter, &subject) {
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
                return Some((Ok(item), (state, new_cursor, filter, subject, tracked)));
            }
        },
    );

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
