use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hkdf::Hkdf;
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use crate::caldav::CalDavClient;
use crate::db;
use crate::sse::{SseBroadcaster, SseMessage};

pub enum SyncTrigger {
    Full,
    Incremental,
    /// Sync only the single calendar at the given href (used by the push webhook).
    Calendar(String),
}

pub struct SyncManager {
    db: db::Db,
    encryption_key: [u8; 32],
    broadcaster: Arc<SseBroadcaster>,
    app_base_url: String,
    sync_interval: Duration,
    tasks: Mutex<HashMap<String, mpsc::UnboundedSender<SyncTrigger>>>,
}

impl SyncManager {
    pub fn new(
        db: db::Db,
        encryption_key: [u8; 32],
        broadcaster: Arc<SseBroadcaster>,
        app_base_url: String,
        sync_interval_mins: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            encryption_key,
            broadcaster,
            app_base_url,
            sync_interval: Duration::from_secs(sync_interval_mins.saturating_mul(60)),
            tasks: Mutex::new(HashMap::new()),
        })
    }

    /// Ensure a background sync task is running for this user.
    /// Returns `true` if a new task was just started, `false` if one was already running.
    pub fn ensure_task_for(self: &Arc<Self>, user_id: &str) -> bool {
        let mut tasks = self.tasks.lock().unwrap();
        if tasks.contains_key(user_id) {
            return false;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tasks.insert(user_id.to_string(), tx);
        drop(tasks);

        let mgr = Arc::clone(self);
        let uid = user_id.to_string();
        tokio::spawn(async move {
            mgr.run_user_loop(uid, rx).await;
        });
        true
    }

    /// Send a trigger to an existing user task (no-op if task not running).
    pub fn trigger(&self, user_id: &str, trigger: SyncTrigger) {
        let tasks = self.tasks.lock().unwrap();
        if let Some(tx) = tasks.get(user_id) {
            let _ = tx.send(trigger);
        }
    }

    async fn run_user_loop(self: Arc<Self>, user_id: String, mut rx: mpsc::UnboundedReceiver<SyncTrigger>) {
        // Initial population on first login
        if let Err(e) = self.sync_user(&user_id, true).await {
            eprintln!("[sync] initial sync failed for {user_id}: {e}");
        }

        loop {
            tokio::select! {
                _ = sleep(self.sync_interval) => {
                    if let Err(e) = self.sync_user(&user_id, false).await {
                        eprintln!("[sync] periodic sync failed for {user_id}: {e}");
                    }
                }
                msg = rx.recv() => {
                    match msg {
                        Some(SyncTrigger::Full) => {
                            if let Err(e) = self.sync_user(&user_id, true).await {
                                eprintln!("[sync] full sync failed for {user_id}: {e}");
                            }
                        }
                        Some(SyncTrigger::Incremental) => {
                            if let Err(e) = self.sync_user(&user_id, false).await {
                                eprintln!("[sync] incremental sync failed for {user_id}: {e}");
                            }
                        }
                        Some(SyncTrigger::Calendar(calendar_href)) => {
                            if let Err(e) = self.sync_single_calendar(&user_id, &calendar_href).await {
                                eprintln!("[sync] single-calendar sync failed for {calendar_href}: {e}");
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        // Remove from task map so ensure_task_for can restart it
        let mut tasks = self.tasks.lock().unwrap();
        tasks.remove(&user_id);
    }

    /// Sync a single calendar by href — used by the push webhook so we don't
    /// trigger ctag-fallback full-fetches on calendars that haven't changed.
    async fn sync_single_calendar(&self, user_id: &str, calendar_href: &str) -> anyhow::Result<()> {
        let settings = db::get_settings(&self.db, user_id, &self.encryption_key).await?;
        let Some(settings) = settings else { return Ok(()); };
        let client = CalDavClient::new(
            settings.caldav_base_url,
            settings.caldav_username,
            settings.caldav_password,
        )?;
        self.sync_calendar(&client, user_id, calendar_href).await
    }

    async fn sync_user(&self, user_id: &str, full: bool) -> anyhow::Result<()> {
        let settings = db::get_settings(&self.db, user_id, &self.encryption_key).await?;
        let Some(settings) = settings else {
            return Ok(()); // no CalDAV credentials configured yet
        };

        let client = CalDavClient::new(
            settings.caldav_base_url,
            settings.caldav_username,
            settings.caldav_password,
        )?;

        // Determine whether we need to re-discover calendars.
        // Always do a full discovery when explicitly requested, or when the cache is empty.
        let cached_empty = if !full {
            db::get_cached_calendars(&self.db, user_id).await?.is_empty()
        } else {
            false
        };
        let do_full = full || cached_empty;

        // On full sync, rediscover the calendar list from CalDAV
        let calendars: Vec<db::CachedCalendar> = if do_full {
            let fresh = client.calendars().await?;
            for cal in &fresh {
                db::upsert_cached_calendar(
                    &self.db,
                    user_id,
                    &cal.href,
                    &cal.name,
                    cal.color.as_deref(),
                )
                .await?;
            }
            self.broadcaster.send(user_id, SseMessage::CalendarsChanged);
            fresh
                .into_iter()
                .map(|c| db::CachedCalendar {
                    href: c.href,
                    name: c.name,
                    color: c.color,
                })
                .collect()
        } else {
            db::get_cached_calendars(&self.db, user_id).await?
        };

        for cal in &calendars {
            if let Err(e) = self.sync_calendar(&client, user_id, &cal.href).await {
                eprintln!("[sync] calendar {} failed: {e}", cal.href);
            }
        }
        Ok(())
    }

    async fn sync_calendar(
        &self,
        client: &CalDavClient,
        user_id: &str,
        calendar_href: &str,
    ) -> anyhow::Result<()> {
        let (stored_ctag, stored_token) =
            db::get_calendar_sync_state(&self.db, user_id, calendar_href).await?;

        // ── Try RFC 6578 incremental sync ───────────────────────────────────────
        if let Some(ref token) = stored_token {
            match client.sync_collection(calendar_href, Some(token)).await {
                Ok(result) => {
                    let apply_result = db::apply_incremental_sync(
                        &self.db,
                        user_id,
                        calendar_href,
                        result.changed,
                        result.deleted,
                        &result.new_token,
                        stored_ctag.as_deref(),
                    )
                    .await?;
                    let total = apply_result.changed.len() + apply_result.deleted.len();
                    if total > 5 {
                        self.broadcaster.send(user_id, SseMessage::EventsRefetch);
                    } else {
                        for ev in &apply_result.deleted {
                            self.broadcaster.send(user_id, SseMessage::EventRemoved {
                                uid: ev.uid.clone(),
                                calendar_href: ev.calendar_href.clone(),
                            });
                        }
                        for ev in &apply_result.changed {
                            self.broadcaster.send(user_id, SseMessage::EventUpdated {
                                uid: ev.uid.clone(),
                                calendar_href: ev.calendar_href.clone(),
                                event_json: None,
                            });
                        }
                    }
                    self.try_subscribe_push(client, user_id, calendar_href).await;
                    return Ok(());
                }
                Err(e) => {
                    eprintln!(
                        "[sync] sync-collection unsupported for {calendar_href}: {e}, falling back to ctag"
                    );
                }
            }
        }

        // ── Fall back to getctag change detection ───────────────────────────────
        let (new_ctag, new_token) = client
            .get_ctag_and_token(calendar_href)
            .await
            .unwrap_or((None, None));

        let need_full_fetch = match (&stored_ctag, &new_ctag) {
            (Some(old), Some(new)) => old != new,
            _ => true, // no stored ctag or server doesn't support it → always fetch
        };

        if need_full_fetch {
            let all_raw = client.fetch_all_raw(calendar_href).await?;
            db::replace_calendar_events(
                &self.db,
                user_id,
                calendar_href,
                all_raw,
                new_ctag.as_deref(),
                new_token.as_deref(),
            )
            .await?;
            self.broadcaster.send(user_id, SseMessage::EventsRefetch);
        }

        self.try_subscribe_push(client, user_id, calendar_href).await;
        Ok(())
    }

    async fn try_subscribe_push(
        &self,
        client: &CalDavClient,
        user_id: &str,
        calendar_href: &str,
    ) {
        let now = chrono::Utc::now().timestamp();
        let renew_threshold = now + self.sync_interval.as_secs() as i64 * 2;

        let existing = db::get_push_subscription(&self.db, user_id, calendar_href)
            .await
            .ok()
            .flatten();
        if let Some((_, expires_at)) = existing {
            if expires_at > renew_threshold {
                return;
            }
        }

        let (p256dh, auth) = match derive_push_keys(&self.encryption_key, user_id, calendar_href) {
            Some(keys) => keys,
            None => {
                eprintln!("[push] key derivation failed for {calendar_href}");
                return;
            }
        };

        let token = Uuid::new_v4().to_string();
        let callback_url = format!(
            "{}/api/push/webhook?token={}",
            self.app_base_url.trim_end_matches('/'),
            token
        );

        match client.register_push(calendar_href, &callback_url, &p256dh, &auth).await {
            Ok(Some(expires_at)) => {
                if let Err(e) = db::upsert_push_subscription(
                    &self.db, user_id, calendar_href, &token, "web-push", expires_at,
                ).await {
                    eprintln!("[push] failed to store subscription for {calendar_href}: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("[push] failed to register push for {calendar_href}: {e}");
            }
        }
    }
}

/// Derive an EC P-256 key pair and auth secret deterministically from the encryption key.
/// Returns `(p256dh_base64url, auth_base64url)` or `None` on failure (extremely unlikely).
fn derive_push_keys(
    encryption_key: &[u8; 32],
    user_id: &str,
    calendar_href: &str,
) -> Option<(String, String)> {
    let salt = format!("{user_id}{calendar_href}");
    let hk = Hkdf::<Sha256>::new(Some(salt.as_bytes()), encryption_key);
    let mut okm = [0u8; 48];
    hk.expand(b"caley-push-ecdh-v1", &mut okm).ok()?;

    let secret_key = SecretKey::from_bytes((&okm[..32]).into()).ok()?;
    let public_key = secret_key.public_key();
    // Uncompressed EC point: 04 || x || y = 65 bytes
    let encoded = public_key.to_encoded_point(false);

    let p256dh = URL_SAFE_NO_PAD.encode(encoded.as_bytes());
    let auth = URL_SAFE_NO_PAD.encode(&okm[32..48]);
    Some((p256dh, auth))
}
