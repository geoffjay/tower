//! Live data: the `/v1/events` SSE subscription (plan T1.2, D§11.1).
//!
//! One connection for the whole TUI. The server frames each batch as
//! `id: <seq>` + one JSON event per `data:` line; the last id seen is the
//! resume cursor. On any failure the loop reconnects with backoff and
//! `?cursor=` so the event log replays the gap.

use std::time::Duration;

use futures::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tower_client::Client;
use tower_core::Event;

/// One dispatched SSE event (blank-line terminated).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Frame {
    pub id: Option<i64>,
    pub data: String,
}

impl Frame {
    /// Batch payload → events (one JSON object per line). Lines that don't
    /// decode — e.g. an event kind newer than this client — are skipped.
    pub fn events(&self) -> Vec<Event> {
        self.data
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
}

/// Incremental SSE parser: bytes in, complete frames out. Chunks may split
/// lines (and UTF-8 sequences) anywhere.
#[derive(Debug, Default)]
pub struct Parser {
    buf: Vec<u8>,
    frame: Frame,
    has_data: bool,
}

impl Parser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if self.has_data || self.frame.id.is_some() {
                    out.push(std::mem::take(&mut self.frame));
                }
                self.has_data = false;
                continue;
            }
            if line.starts_with(':') {
                continue; // comment / keep-alive
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line, ""),
            };
            match field {
                "data" => {
                    if self.has_data {
                        self.frame.data.push('\n');
                    }
                    self.frame.data.push_str(value);
                    self.has_data = true;
                }
                "id" => self.frame.id = value.trim().parse().ok(),
                _ => {} // `event:` is always `tower-batch`; `retry:` unused
            }
        }
        out
    }
}

/// Feed → app messages.
#[derive(Debug)]
pub enum FeedMsg {
    /// Stream open (first connect or reconnect): resync the snapshot.
    Connected,
    Events(Vec<Event>),
    /// Stream lost; the loop is reconnecting.
    Down(String),
}

/// `/v1/events` path for a (re)connect.
pub fn events_path(cursor: Option<i64>) -> String {
    match cursor {
        Some(c) => format!("/v1/events?cursor={c}"),
        None => "/v1/events".into(),
    }
}

/// Run forever (until the receiver is dropped), reconnecting with backoff.
pub async fn run<M: From<FeedMsg> + Send + 'static>(client: Client, tx: UnboundedSender<M>) {
    let mut cursor: Option<i64> = None;
    let mut backoff = Duration::from_millis(500);
    loop {
        match client.stream(&events_path(cursor)).await {
            Ok(resp) => {
                backoff = Duration::from_millis(500);
                if tx.send(FeedMsg::Connected.into()).is_err() {
                    return;
                }
                let mut parser = Parser::default();
                let mut body = resp.bytes_stream();
                let err = loop {
                    match body.next().await {
                        Some(Ok(chunk)) => {
                            for frame in parser.push(&chunk) {
                                if let Some(id) = frame.id {
                                    cursor = Some(id);
                                }
                                let events = frame.events();
                                if !events.is_empty()
                                    && tx.send(FeedMsg::Events(events).into()).is_err()
                                {
                                    return;
                                }
                            }
                        }
                        Some(Err(e)) => break e.to_string(),
                        None => break "server closed the stream".to_string(),
                    }
                };
                if tx.send(FeedMsg::Down(err).into()).is_err() {
                    return;
                }
            }
            Err(e) => {
                if tx.send(FeedMsg::Down(format!("{e:#}")).into()).is_err() {
                    return;
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_survive_arbitrary_chunk_splits() {
        let wire = "event: tower-batch\nid: 7\ndata: {\"a\":1}\ndata: {\"b\":\"é\"}\n\n: keep-alive\n\nid: 8\ndata: \n\n";
        let bytes = wire.as_bytes();
        for split in 1..bytes.len() {
            let mut p = Parser::default();
            let mut frames = p.push(&bytes[..split]);
            frames.extend(p.push(&bytes[split..]));
            assert_eq!(
                frames,
                vec![
                    Frame {
                        id: Some(7),
                        data: "{\"a\":1}\n{\"b\":\"é\"}".into()
                    },
                    // an all-filtered batch still advances the cursor
                    Frame {
                        id: Some(8),
                        data: String::new()
                    },
                ],
                "split at {split}"
            );
        }
    }

    #[test]
    fn undecodable_lines_are_skipped_not_fatal() {
        let f = Frame {
            id: Some(3),
            data: concat!(
                r#"{"seq":2,"ts":1,"kind":"from.the.future","payload":{}}"#,
                "\n",
                r#"{"seq":3,"ts":1,"kind":"agent.created","subject_type":"agent","subject_id":"a1","payload":{}}"#
            )
            .into(),
        };
        let ev = f.events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].seq, 3);
    }
}
