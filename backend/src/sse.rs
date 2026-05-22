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
    /// One or more contacts changed — client should re-fetch /api/contacts
    ContactsRefetch,
    /// A single contact was deleted
    ContactRemoved { uid: String, addressbook_href: String },
    /// A single contact was created or updated (contact_json is inline to avoid a roundtrip)
    ContactUpdated { uid: String, addressbook_href: String, contact_json: Option<String> },
    /// One or more emails changed — client should re-fetch the mailbox
    EmailsRefetch { account_id: String, mailbox: String },
    /// A single email was deleted
    EmailRemoved { account_id: String, uid: u32, mailbox: String },
    /// A single email was created or its flags changed
    EmailUpdated { account_id: String, uid: u32, mailbox: String, flags: Vec<String> },
    /// The account list changed (account added or removed)
    AccountsChanged,
    /// The mailbox list for an account changed
    MailboxesChanged { account_id: String },
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

    // Ensure CalDAV and email sync tasks are running for this user.
    state.sync_manager.ensure_task_for(&user.user_id);
    state.email_sync_manager.ensure_tasks_for_user(&user.user_id).await;
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
            Ok(SseMessage::ContactsRefetch) => {
                Some(Event::default().event("contacts_refetch").data("1"))
            }
            Ok(SseMessage::ContactRemoved { uid, addressbook_href }) => {
                Some(Event::default().event("contact_removed").data(
                    serde_json::json!({ "uid": uid, "addressbook_href": addressbook_href }).to_string()
                ))
            }
            Ok(SseMessage::ContactUpdated { uid, addressbook_href, contact_json }) => {
                let data = if let Some(contact) = contact_json {
                    let parsed: serde_json::Value = serde_json::from_str(&contact).unwrap_or_default();
                    serde_json::json!({ "uid": uid, "addressbook_href": addressbook_href, "contact": parsed })
                        .to_string()
                } else {
                    serde_json::json!({ "uid": uid, "addressbook_href": addressbook_href }).to_string()
                };
                Some(Event::default().event("contact_updated").data(data))
            }
            Ok(SseMessage::EmailsRefetch { account_id, mailbox }) => {
                Some(Event::default().event("emails_refetch").data(
                    serde_json::json!({ "account_id": account_id, "mailbox": mailbox }).to_string(),
                ))
            }
            Ok(SseMessage::EmailRemoved { account_id, uid, mailbox }) => {
                Some(Event::default().event("email_removed").data(
                    serde_json::json!({ "account_id": account_id, "uid": uid, "mailbox": mailbox }).to_string(),
                ))
            }
            Ok(SseMessage::EmailUpdated { account_id, uid, mailbox, flags }) => {
                Some(Event::default().event("email_updated").data(
                    serde_json::json!({ "account_id": account_id, "uid": uid, "mailbox": mailbox, "flags": flags }).to_string(),
                ))
            }
            Ok(SseMessage::AccountsChanged) => {
                Some(Event::default().event("accounts_changed").data("1"))
            }
            Ok(SseMessage::MailboxesChanged { account_id }) => {
                Some(Event::default().event("mailboxes_changed").data(
                    serde_json::json!({ "account_id": account_id }).to_string(),
                ))
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

    // Check calendar push subscriptions first
    match db::get_user_and_calendar_from_push_token(&state.db, &token).await {
        Ok(Some((user_id, calendar_href))) => {
            eprintln!("[push] webhook matched user {user_id} calendar {calendar_href}, triggering targeted sync");
            state.sync_manager.trigger(&user_id, SyncTrigger::Calendar(calendar_href));
            return axum::http::StatusCode::CREATED.into_response();
        }
        Ok(None) => {}
        Err(e) => eprintln!("[push] calendar token lookup error: {e}"),
    }

    // Check contact push subscriptions
    match db::get_user_and_addressbook_from_push_token(&state.db, &token).await {
        Ok(Some((user_id, addressbook_href))) => {
            eprintln!("[push] webhook matched user {user_id} addressbook {addressbook_href}, triggering contact sync");
            state.sync_manager.trigger(&user_id, SyncTrigger::Addressbook(addressbook_href));
            axum::http::StatusCode::CREATED.into_response()
        }
        _ => {
            eprintln!("[push] webhook token not found in any push_subscriptions table");
            (axum::http::StatusCode::UNAUTHORIZED, "unknown token").into_response()
        }
    }
}
