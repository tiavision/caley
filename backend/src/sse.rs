use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};

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
    EventRemoved {
        account_id: String,
        uid: String,
        calendar_href: String,
    },
    /// A single event was created or updated
    EventUpdated {
        account_id: String,
        uid: String,
        calendar_href: String,
        event_json: Option<String>,
    },
    /// One or more contacts changed — client should re-fetch /api/contacts
    ContactsRefetch,
    /// A single contact was deleted
    ContactRemoved {
        account_id: String,
        uid: String,
        addressbook_href: String,
    },
    /// A single contact was created or updated (contact_json is inline to avoid a roundtrip)
    ContactUpdated {
        account_id: String,
        uid: String,
        addressbook_href: String,
        contact_json: Option<String>,
    },
    /// One or more emails changed — client should re-fetch the mailbox
    EmailsRefetch { account_id: String, mailbox: String },
    /// A single email was deleted
    EmailRemoved {
        account_id: String,
        uid: u32,
        mailbox: String,
    },
    /// A single email was created or its flags changed
    EmailUpdated {
        account_id: String,
        uid: u32,
        mailbox: String,
        flags: Vec<String>,
    },
    /// A newly delivered unread inbox email suitable for a client notification
    NewEmail {
        account_id: String,
        uid: u32,
        mailbox: String,
        from_addr: Option<String>,
        subject: Option<String>,
    },
    /// The account list changed (account added or removed)
    AccountsChanged,
    /// The mailbox list for an account changed
    MailboxesChanged { account_id: String },
    /// A mailbox sync has started
    SyncStarted { account_id: String, mailbox: String },
    /// A mailbox sync has completed (or failed — spinner must always stop)
    SyncCompleted { account_id: String, mailbox: String },
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

    pub(crate) fn subscribe(&self, user_id: &str) -> broadcast::Receiver<SseMessage> {
        let mut channels = self.channels.lock().unwrap();
        let tx = channels
            .entry(user_id.to_string())
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0);
        tx.subscribe()
    }
}

/// The live updates of the signed-in user (server-sent events; a `heartbeat` comment every 30 s).
/// Starts the user's CalDAV and email sync tasks.
#[utoipa::path(
    get,
    path = "/events/stream",
    responses((
        status = 200,
        body = String,
        content_type = "text/event-stream",
        description = "Server-sent events, each an `event:` name with a `data:` line. With data `1` \
(re-fetch): `events_refetch` (/api/events), `calendars_changed` (/api/calendars), `contacts_refetch` \
(/api/contacts), `accounts_changed` (the email accounts). With JSON data: \
`event_updated` `{account_id, uid, calendar_href, event?: EventJson}`, \
`event_removed` `{account_id, uid, calendar_href}`, \
`contact_updated` `{account_id, uid, addressbook_href, contact?: ContactJson}`, \
`contact_removed` `{account_id, uid, addressbook_href}`, \
`emails_refetch` `{account_id, mailbox}`, \
`email_updated` `{account_id, uid: number, mailbox, flags: string[]}`, \
`email_removed` `{account_id, uid: number, mailbox}`, \
`new_email` `{account_id, uid: number, mailbox, from: string|null, subject: string|null}`, \
`mailboxes_changed` `{account_id}`, \
`sync_started` and `sync_completed` `{account_id, mailbox}`."
    ))
)]
pub async fn events_stream(
    axum::extract::State(state): axum::extract::State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    // Subscribe before triggering sync so we never miss the completion event.
    let rx = state.sse_broadcaster.subscribe(&user.user_id);

    // Ensure CalDAV and email sync tasks are running for this user.
    state.sync_manager.ensure_task_for(&user.user_id);
    if !state.demo_mode {
        state
            .email_sync_manager
            .ensure_tasks_for_user(&user.user_id)
            .await;
    }
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        let event: Option<Event> = match msg {
            Ok(SseMessage::EventsRefetch) => {
                Some(Event::default().event("events_refetch").data("1"))
            }
            Ok(SseMessage::CalendarsChanged) => {
                Some(Event::default().event("calendars_changed").data("1"))
            }
            Ok(SseMessage::EventRemoved { account_id, uid, calendar_href }) => {
                Some(Event::default().event("event_removed").data(
                    serde_json::json!({ "account_id": account_id, "uid": uid, "calendar_href": calendar_href }).to_string(),
                ))
            }
            Ok(SseMessage::EventUpdated { account_id, uid, calendar_href, event_json }) => {
                let data = if let Some(event) = event_json {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&event).unwrap_or_default();
                    serde_json::json!({ "account_id": account_id, "uid": uid, "calendar_href": calendar_href, "event": parsed })
                        .to_string()
                } else {
                    serde_json::json!({ "account_id": account_id, "uid": uid, "calendar_href": calendar_href }).to_string()
                };
                Some(Event::default().event("event_updated").data(data))
            }
            Ok(SseMessage::ContactsRefetch) => {
                Some(Event::default().event("contacts_refetch").data("1"))
            }
            Ok(SseMessage::ContactRemoved { account_id, uid, addressbook_href }) => {
                Some(Event::default().event("contact_removed").data(
                    serde_json::json!({ "account_id": account_id, "uid": uid, "addressbook_href": addressbook_href }).to_string()
                ))
            }
            Ok(SseMessage::ContactUpdated { account_id, uid, addressbook_href, contact_json }) => {
                let data = if let Some(contact) = contact_json {
                    let parsed: serde_json::Value = serde_json::from_str(&contact).unwrap_or_default();
                    serde_json::json!({ "account_id": account_id, "uid": uid, "addressbook_href": addressbook_href, "contact": parsed })
                        .to_string()
                } else {
                    serde_json::json!({ "account_id": account_id, "uid": uid, "addressbook_href": addressbook_href }).to_string()
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
            Ok(SseMessage::NewEmail { account_id, uid, mailbox, from_addr, subject }) => {
                Some(Event::default().event("new_email").data(
                    serde_json::json!({
                        "account_id": account_id,
                        "uid": uid,
                        "mailbox": mailbox,
                        "from": from_addr,
                        "subject": subject,
                    }).to_string(),
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
            Ok(SseMessage::SyncStarted { account_id, mailbox }) => {
                Some(Event::default().event("sync_started").data(
                    serde_json::json!({ "account_id": account_id, "mailbox": mailbox }).to_string(),
                ))
            }
            Ok(SseMessage::SyncCompleted { account_id, mailbox }) => {
                Some(Event::default().event("sync_completed").data(
                    serde_json::json!({ "account_id": account_id, "mailbox": mailbox }).to_string(),
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

#[derive(serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PushWebhookQuery {
    /// the push subscription's token (of a calendar or an address book)
    token: Option<String>,
}

/// Webhook endpoint for WebDAV-Push notifications.
/// Validates the per-calendar token, then triggers an incremental sync.
#[utoipa::path(
    post,
    path = "/push/webhook",
    params(PushWebhookQuery),
    responses(
        (status = 201, description = "accepted (also for an unknown token, so tokens cannot be probed)"),
        (status = 400, description = "missing token (plain text)")
    )
)]
pub async fn push_webhook(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Query(params): axum::extract::Query<PushWebhookQuery>,
) -> impl IntoResponse {
    use crate::sync::SyncTrigger;

    let token = match params.token {
        Some(t) => t,
        None => {
            eprintln!("[push] webhook called with no token");
            return (axum::http::StatusCode::BAD_REQUEST, "missing token").into_response();
        }
    };

    eprintln!("[push] webhook called with token {token}");

    // Check calendar push subscriptions first
    match db::get_user_and_calendar_from_push_token(&state.db, &token).await {
        Ok(Some((user_id, account_id, calendar_href))) => {
            eprintln!(
                "[push] webhook matched user {user_id} calendar {calendar_href}, triggering targeted sync"
            );
            state.sync_manager.trigger(
                &user_id,
                SyncTrigger::Calendar {
                    account_id,
                    href: calendar_href,
                },
            );
            return axum::http::StatusCode::CREATED.into_response();
        }
        Ok(None) => {}
        Err(e) => eprintln!("[push] calendar token lookup error: {e}"),
    }

    // Check contact push subscriptions
    match db::get_user_and_addressbook_from_push_token(&state.db, &token).await {
        Ok(Some((user_id, account_id, addressbook_href))) => {
            eprintln!(
                "[push] webhook matched user {user_id} addressbook {addressbook_href}, triggering contact sync"
            );
            state.sync_manager.trigger(
                &user_id,
                SyncTrigger::Addressbook {
                    account_id,
                    href: addressbook_href,
                },
            );
            axum::http::StatusCode::CREATED.into_response()
        }
        _ => {
            // Return the same status as a matched token so this public endpoint
            // can't be used to enumerate valid push tokens.
            eprintln!("[push] webhook token not found in any push_subscriptions table");
            axum::http::StatusCode::CREATED.into_response()
        }
    }
}

/// This module's routes that need a signed-in user, relative to `/api` (api.rs).
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(events_stream))
}

/// This module's public routes, relative to `/api` (api.rs).
pub fn public_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(push_webhook))
}
