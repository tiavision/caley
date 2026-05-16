use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

use crate::oidc::{AppState, ExtractUserId};

const CHANNEL_CAPACITY: usize = 64;

#[derive(Clone, Debug)]
pub enum SseMessage {
    /// One or more events changed — client should re-fetch /api/events
    EventsRefetch,
    /// Calendar list changed — client should re-fetch /api/calendars
    CalendarsChanged,
    /// A single event was deleted
    EventRemoved { uid: String, calendar_href: String },
}

pub struct SseBroadcaster {
    channels: Mutex<HashMap<String, broadcast::Sender<SseMessage>>>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
        }
    }

    pub fn send(&self, user_id: &str, msg: SseMessage) {
        let channels = self.channels.lock().unwrap();
        if let Some(tx) = channels.get(user_id) {
            let _ = tx.send(msg);
        }
    }

    fn subscribe(&self, user_id: &str) -> broadcast::Receiver<SseMessage> {
        let mut channels = self.channels.lock().unwrap();
        let tx = channels
            .entry(user_id.to_string())
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0);
        tx.subscribe()
    }
}

pub async fn events_stream(
    axum::extract::State(state): axum::extract::State<AppState>,
    ExtractUserId(user_id): ExtractUserId,
) -> impl IntoResponse {
    let rx = state.sse_broadcaster.subscribe(&user_id);
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        let event: Option<Event> = match msg {
            Ok(SseMessage::EventsRefetch) => {
                Some(Event::default().event("events_refetch").data(""))
            }
            Ok(SseMessage::CalendarsChanged) => {
                Some(Event::default().event("calendars_changed").data(""))
            }
            Ok(SseMessage::EventRemoved { uid, calendar_href }) => {
                Some(Event::default().event("event_removed").data(format!("{uid}|{calendar_href}")))
            }
            Err(_) => None,
        };
        event.map(Ok::<_, std::convert::Infallible>)
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("heartbeat"),
    )
}

/// Webhook endpoint for WebDAV-Push notifications.
/// Validates the per-calendar token, then triggers an incremental sync.
pub async fn push_webhook(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    use crate::sync::SyncTrigger;

    let token = match params.get("token") {
        Some(t) => t.clone(),
        None => return (axum::http::StatusCode::BAD_REQUEST, "missing token").into_response(),
    };

    // Look up which user+calendar this token belongs to
    let db = state.db.lock().unwrap();
    let result = db.query_row(
        "SELECT user_id FROM push_subscriptions WHERE webhook_token = ?1",
        rusqlite::params![token],
        |row| row.get::<_, String>(0),
    );
    drop(db);

    match result {
        Ok(user_id) => {
            state.sync_manager.trigger(&user_id, SyncTrigger::Incremental);
            axum::http::StatusCode::OK.into_response()
        }
        Err(_) => (axum::http::StatusCode::UNAUTHORIZED, "unknown token").into_response(),
    }
}
