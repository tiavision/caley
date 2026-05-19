use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

use crate::db;
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;

const CHANNEL_CAPACITY: usize = 64;

#[derive(Clone, Debug)]
pub enum SseMessage {
    /// One or more events changed — client should re-fetch /api/events
    EventsRefetch,
    /// Calendar list changed — client should re-fetch /api/calendars
    CalendarsChanged,
    /// A single event was deleted
    EventRemoved { uid: String, calendar_href: String },
    /// A single event was created or updated
    EventUpdated { uid: String, calendar_href: String, event_json: Option<String> },
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
    user: CurrentUser,
) -> impl IntoResponse {
    // Subscribe before triggering sync so we never miss the completion event.
    let rx = state.sse_broadcaster.subscribe(&user.user_id);

    // Ensure the sync task is running.
    // If the task was newly started it immediately runs an initial full sync
    // in run_user_loop, so we must NOT also queue a trigger — that would
    // cause two back-to-back syncs and duplicate events_refetch pushes.
    // If the task was already running, queue an incremental (or full) sync
    // so fresh changes are picked up on page reload.
    let just_started = state.sync_manager.ensure_task_for(&user.user_id);
    if !just_started {
        let has_cached = db::get_cached_calendars(&state.db, &user.user_id)
            .await
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        state.sync_manager.trigger(
            &user.user_id,
            if has_cached { SyncTrigger::Incremental } else { SyncTrigger::Full },
        );
    }
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        let event: Option<Event> = match msg {
            Ok(SseMessage::EventsRefetch) => {
                Some(Event::default().event("events_refetch").data("1"))
            }
            Ok(SseMessage::CalendarsChanged) => {
                Some(Event::default().event("calendars_changed").data("1"))
            }
            Ok(SseMessage::EventRemoved { uid, calendar_href }) => {
                Some(Event::default().event("event_removed").data(format!("{uid}|{calendar_href}")))
            }
            Ok(SseMessage::EventUpdated { uid, calendar_href, event_json }) => {
                let data = if let Some(event) = event_json {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&event).unwrap_or_default();
                    serde_json::json!({ "uid": uid, "calendar_href": calendar_href, "event": parsed })
                        .to_string()
                } else {
                    serde_json::json!({ "uid": uid, "calendar_href": calendar_href }).to_string()
                };
                Some(Event::default().event("event_updated").data(data))
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
        None => {
            eprintln!("[push] webhook called with no token");
            return (axum::http::StatusCode::BAD_REQUEST, "missing token").into_response();
        }
    };

    eprintln!("[push] webhook called with token {token}");

    // Look up which user+calendar this token belongs to
    match db::get_user_and_calendar_from_push_token(&state.db, &token).await {
        Ok(Some((user_id, calendar_href))) => {
            eprintln!("[push] webhook matched user {user_id} calendar {calendar_href}, triggering targeted sync");
            state.sync_manager.trigger(&user_id, SyncTrigger::Calendar(calendar_href));
            axum::http::StatusCode::CREATED.into_response()
        }
        _ => {
            eprintln!("[push] webhook token not found in push_subscriptions");
            (axum::http::StatusCode::UNAUTHORIZED, "unknown token").into_response()
        }
    }
}
