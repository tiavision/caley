use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::caldav::CalDavClient;
use crate::carddav;
use crate::db;
use crate::search::SearchMode;
use crate::sse::{SseBroadcaster, SseMessage};

pub enum SyncTrigger {
    Full,
    Incremental,
    /// Sync only the single calendar at the given href (used by the push webhook).
    Calendar { account_id: String, href: String },
    /// Sync only the single addressbook at the given href (used by the push webhook).
    Addressbook { account_id: String, href: String },
}

pub struct SyncManager {
    db: db::Db,
    encryption_key: [u8; 32],
    broadcaster: Arc<SseBroadcaster>,
    app_base_url: String,
    sync_interval: Duration,
    search_mode: SearchMode,
    tasks: Mutex<HashMap<String, mpsc::UnboundedSender<SyncTrigger>>>,
    demo_mode: bool,
}

impl SyncManager {
    pub fn new(
        db: db::Db,
        encryption_key: [u8; 32],
        broadcaster: Arc<SseBroadcaster>,
        app_base_url: String,
        sync_interval_mins: u64,
        search_mode: SearchMode,
        demo_mode: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            encryption_key,
            broadcaster,
            app_base_url,
            sync_interval: Duration::from_secs(sync_interval_mins.saturating_mul(60)),
            search_mode,
            tasks: Mutex::new(HashMap::new()),
            demo_mode,
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

    async fn run_user_loop(
        self: Arc<Self>,
        user_id: String,
        mut rx: mpsc::UnboundedReceiver<SyncTrigger>,
    ) {
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
                        Some(SyncTrigger::Calendar { account_id, href }) => {
                            if let Err(e) = self.sync_single_calendar(&user_id, &account_id, &href).await {
                                eprintln!("[sync] single-calendar sync failed for {href}: {e}");
                            }
                        }
                        Some(SyncTrigger::Addressbook { account_id, href }) => {
                            if let Err(e) = self.sync_single_addressbook(&user_id, &account_id, &href).await {
                                eprintln!("[sync] single-addressbook sync failed for {href}: {e}");
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
    async fn sync_single_calendar(&self, user_id: &str, account_id: &str, calendar_href: &str) -> anyhow::Result<()> {
        let account = db::get_caldav_account(&self.db, user_id, account_id, &self.encryption_key).await?;
        let Some(account) = account else {
            return Ok(());
        };
        let client = CalDavClient::new(
            account.caldav_base_url,
            account.caldav_username,
            account.caldav_password,
        )?;
        self.sync_calendar(&client, user_id, account_id, calendar_href).await
    }

    async fn sync_user(&self, user_id: &str, _full: bool) -> anyhow::Result<()> {
        if self.demo_mode {
            return Ok(());
        }
        let accounts = db::list_caldav_accounts(&self.db, user_id, &self.encryption_key).await?;
        if accounts.is_empty() {
            return Ok(()); // no CalDAV credentials configured yet
        }

        for account in accounts {
            let account_id = account.account_id.clone();
            let client = CalDavClient::new(
                account.caldav_base_url,
                account.caldav_username,
                account.caldav_password,
            )?;

        // Rediscover the calendar list from CalDAV on every sync — a single PROPFIND,
        // so the cost is low. This ensures new calendars added on the server are picked
        // up within one sync interval and get push subscriptions registered.
        let calendars: Vec<db::CachedCalendar> = match client.calendars().await {
            Ok(fresh) if !fresh.is_empty() => {
                let fresh_hrefs: HashSet<String> = fresh.iter().map(|c| c.href.clone()).collect();
                let old_cached = db::get_cached_calendars(&self.db, user_id)
                    .await?
                    .into_iter()
                    .filter(|c| c.account_id == account_id)
                    .collect::<Vec<_>>();
                let old_hrefs: HashSet<String> =
                    old_cached.iter().map(|c| c.href.clone()).collect();
                for cal in &fresh {
                    db::upsert_cached_calendar(
                        &self.db,
                        &account_id,
                        &cal.href,
                        &cal.name,
                        cal.color.as_deref(),
                    )
                    .await?;
                }
                db::remove_stale_calendars(&self.db, &account_id, &fresh_hrefs, &self.encryption_key)
                    .await?;
                if fresh_hrefs != old_hrefs {
                    self.broadcaster.send(user_id, SseMessage::CalendarsChanged);
                }
                fresh
                    .into_iter()
                    .map(|c| db::CachedCalendar {
                        href: c.href,
                        name: c.name,
                        color: c.color,
                        account_id: account_id.clone(),
                    })
                    .collect()
            }
            Ok(_) => {
                eprintln!("[sync] calendars() returned empty for {user_id}, falling back to cache");
                db::get_cached_calendars(&self.db, user_id)
                    .await?
                    .into_iter()
                    .filter(|c| c.account_id == account_id)
                    .collect()
            }
            Err(e) => {
                eprintln!("[sync] calendars() failed for {user_id}: {e}");
                db::get_cached_calendars(&self.db, user_id)
                    .await?
                    .into_iter()
                    .filter(|c| c.account_id == account_id)
                    .collect()
            }
        };

        for cal in &calendars {
            if let Err(e) = self.sync_calendar(&client, user_id, &account_id, &cal.href).await {
                let err_str = e.to_string();
                if err_str.contains("404") {
                    eprintln!(
                        "[sync] calendar {} returned 404, removing from cache",
                        cal.href
                    );
                    if let Err(remove_err) = db::delete_calendar_cache(
                        &self.db,
                        &account_id,
                        &cal.href,
                        &self.encryption_key,
                    )
                    .await
                    {
                        eprintln!(
                            "[sync] failed to remove stale calendar {}: {remove_err}",
                            cal.href
                        );
                    }
                    self.broadcaster.send(user_id, SseMessage::CalendarsChanged);
                } else {
                    eprintln!("[sync] calendar {} failed: {e}", cal.href);
                }
            }
        }

        // Sync contacts alongside calendars
        if let Err(e) = self.sync_contacts(&client, user_id, &account_id).await {
            eprintln!("[sync] contact sync failed for {user_id}: {e}");
        }
        }

        Ok(())
    }

    async fn sync_contacts(&self, client: &CalDavClient, user_id: &str, account_id: &str) -> anyhow::Result<()> {
        // Rediscover addressbooks on every sync — ensures new ones are picked up within
        // one sync interval and get push subscriptions registered.
        let addressbooks: Vec<db::CachedAddressbook> = match carddav::discover_addressbooks(client).await {
            Ok(fresh) if !fresh.is_empty() => {
                let fresh_hrefs: HashSet<String> = fresh.iter().map(|a| a.href.clone()).collect();
                for ab in &fresh {
                    if let Err(e) =
                        db::upsert_cached_addressbook(&self.db, account_id, &ab.href, &ab.name).await
                    {
                        eprintln!("[sync] upsert_cached_addressbook error: {e}");
                    }
                }
                if let Err(e) = db::remove_stale_addressbooks(&self.db, account_id, &fresh_hrefs).await
                {
                    eprintln!("[sync] remove_stale_addressbooks error: {e}");
                }
                fresh
                    .into_iter()
                    .map(|a| db::CachedAddressbook {
                        account_id: account_id.to_string(),
                        href: a.href,
                        name: a.name,
                    })
                    .collect()
            }
            Ok(_) => {
                eprintln!(
                    "[sync] discover_addressbooks returned empty for {user_id}, falling back to cache"
                );
                db::get_cached_addressbooks(&self.db, user_id)
                    .await?
                    .into_iter()
                    .filter(|a| a.account_id == account_id)
                    .collect()
            }
            Err(e) => {
                eprintln!("[sync] discover_addressbooks failed for {user_id}: {e}");
                db::get_cached_addressbooks(&self.db, user_id)
                    .await?
                    .into_iter()
                    .filter(|a| a.account_id == account_id)
                    .collect()
            }
        };

        for ab in &addressbooks {
            if let Err(e) = self.sync_addressbook(client, user_id, account_id, &ab.href).await {
                eprintln!("[sync] addressbook {} failed: {e}", ab.href);
            }
        }
        Ok(())
    }

    async fn sync_single_addressbook(
        &self,
        user_id: &str,
        account_id: &str,
        addressbook_href: &str,
    ) -> anyhow::Result<()> {
        let account = db::get_caldav_account(&self.db, user_id, account_id, &self.encryption_key).await?;
        let Some(account) = account else {
            return Ok(());
        };
        let client = CalDavClient::new(
            account.caldav_base_url,
            account.caldav_username,
            account.caldav_password,
        )?;
        self.sync_addressbook(&client, user_id, account_id, addressbook_href)
            .await
    }

    async fn sync_addressbook(
        &self,
        client: &CalDavClient,
        user_id: &str,
        account_id: &str,
        addressbook_href: &str,
    ) -> anyhow::Result<()> {
        let (stored_ctag, stored_token) =
            db::get_addressbook_sync_state(&self.db, account_id, addressbook_href).await?;

        // Try RFC 6578 incremental sync
        if let Some(ref token) = stored_token {
            match carddav::sync_addressbook_collection(client, addressbook_href, Some(token)).await
            {
                Ok(result) => {
                    let apply = db::apply_incremental_contact_sync(
                        &self.db,
                        account_id,
                        addressbook_href,
                        result.changed,
                        result.deleted,
                        &result.new_token,
                        stored_ctag.as_deref(),
                    )
                    .await?;

                    let total = apply.changed.len() + apply.deleted.len();
                    if total > 5 {
                        self.broadcaster.send(user_id, SseMessage::ContactsRefetch);
                    } else {
                        for cr in &apply.deleted {
                            self.broadcaster.send(
                                user_id,
                                SseMessage::ContactRemoved {
                                    uid: cr.uid.clone(),
                                    account_id: cr.account_id.clone(),
                                    addressbook_href: cr.addressbook_href.clone(),
                                },
                            );
                        }
                        for cr in &apply.changed {
                            let contact_json = self
                                .load_contact_json(&cr.account_id, &cr.addressbook_href, &cr.uid)
                                .await;
                            self.broadcaster.send(
                                user_id,
                                SseMessage::ContactUpdated {
                                    uid: cr.uid.clone(),
                                    account_id: cr.account_id.clone(),
                                    addressbook_href: cr.addressbook_href.clone(),
                                    contact_json,
                                },
                            );
                        }
                    }
                    self.try_subscribe_push_contact(client, account_id, addressbook_href)
                        .await;
                    return Ok(());
                }
                Err(e) => {
                    eprintln!(
                        "[sync] contact sync-collection unsupported for {addressbook_href}: {e}, falling back to ctag"
                    );
                }
            }
        }

        // Fall back to ctag
        let (new_ctag, new_token) =
            carddav::get_addressbook_ctag_and_token(client, addressbook_href)
                .await
                .unwrap_or((None, None));

        let need_full = match (&stored_ctag, &new_ctag) {
            (Some(old), Some(new)) => old != new,
            _ => true,
        };

        if need_full {
            let all_raw = carddav::fetch_all_raw_contacts(client, addressbook_href).await?;
            db::replace_addressbook_contacts(
                &self.db,
                account_id,
                addressbook_href,
                all_raw,
                new_ctag.as_deref(),
                new_token.as_deref(),
            )
            .await?;
            self.broadcaster.send(user_id, SseMessage::ContactsRefetch);
        }

        self.try_subscribe_push_contact(client, account_id, addressbook_href)
            .await;
        Ok(())
    }

    async fn load_contact_json(
        &self,
        account_id: &str,
        addressbook_href: &str,
        uid: &str,
    ) -> Option<String> {
        let contact = db::get_contact(&self.db, account_id, addressbook_href, uid)
            .await
            .ok()??;
        let cj = crate::contacts::cached_to_json_pub(&contact);
        serde_json::to_string(&cj).ok()
    }

    async fn try_subscribe_push_contact(
        &self,
        client: &CalDavClient,
        account_id: &str,
        addressbook_href: &str,
    ) {
        let now = chrono::Utc::now().timestamp();
        let renew_threshold = now + self.sync_interval.as_secs() as i64 * 2;

        let existing = db::get_contact_push_subscription(&self.db, account_id, addressbook_href)
            .await
            .ok()
            .flatten();
        if let Some((_, expires_at)) = existing {
            if expires_at > renew_threshold {
                return;
            }
        }

        let (p256dh, auth) = match derive_push_keys(&self.encryption_key, account_id, addressbook_href)
        {
            Some(keys) => keys,
            None => {
                eprintln!("[push] contact key derivation failed for {addressbook_href}");
                return;
            }
        };

        let token = Uuid::new_v4().to_string();
        let callback_url = format!(
            "{}/api/push/webhook?token={}",
            self.app_base_url.trim_end_matches('/'),
            token
        );

        match client
            .register_push(addressbook_href, &callback_url, &p256dh, &auth)
            .await
        {
            Ok(Some(expires_at)) => {
                if let Err(e) = db::upsert_contact_push_subscription(
                    &self.db,
                    account_id,
                    addressbook_href,
                    &token,
                    "web-push",
                    expires_at,
                )
                .await
                {
                    eprintln!(
                        "[push] failed to store contact subscription for {addressbook_href}: {e}"
                    );
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("[push] failed to register contact push for {addressbook_href}: {e}");
            }
        }
    }

    async fn sync_calendar(
        &self,
        client: &CalDavClient,
        user_id: &str,
        account_id: &str,
        calendar_href: &str,
    ) -> anyhow::Result<()> {
        let (stored_ctag, stored_token) =
            db::get_calendar_sync_state(&self.db, account_id, calendar_href).await?;

        // ── Try RFC 6578 incremental sync ───────────────────────────────────────
        if let Some(ref token) = stored_token {
            match client.sync_collection(calendar_href, Some(token)).await {
                Ok(result) => {
                    let apply_result = db::apply_incremental_sync(
                        &self.db,
                        account_id,
                        calendar_href,
                        result.changed,
                        result.deleted,
                        &result.new_token,
                        stored_ctag.as_deref(),
                        &self.search_mode,
                    )
                    .await?;
                    let total = apply_result.changed.len() + apply_result.deleted.len();
                    if total > 5 {
                        self.broadcaster.send(user_id, SseMessage::EventsRefetch);
                    } else {
                        for ev in &apply_result.deleted {
                            self.broadcaster.send(
                                user_id,
                                SseMessage::EventRemoved {
                                    account_id: ev.account_id.clone(),
                                    uid: ev.uid.clone(),
                                    calendar_href: ev.calendar_href.clone(),
                                },
                            );
                        }
                        for ev in &apply_result.changed {
                            self.broadcaster.send(
                                user_id,
                                SseMessage::EventUpdated {
                                    account_id: ev.account_id.clone(),
                                    uid: ev.uid.clone(),
                                    calendar_href: ev.calendar_href.clone(),
                                    event_json: None,
                                },
                            );
                        }
                    }
                    self.try_subscribe_push(client, account_id, calendar_href)
                        .await;
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
                account_id,
                calendar_href,
                all_raw,
                new_ctag.as_deref(),
                new_token.as_deref(),
                &self.search_mode,
            )
            .await?;
            self.broadcaster.send(user_id, SseMessage::EventsRefetch);
        }

        self.try_subscribe_push(client, account_id, calendar_href)
            .await;
        Ok(())
    }

    async fn try_subscribe_push(&self, client: &CalDavClient, account_id: &str, calendar_href: &str) {
        let now = chrono::Utc::now().timestamp();
        let renew_threshold = now + self.sync_interval.as_secs() as i64 * 2;

        let existing = db::get_push_subscription(&self.db, account_id, calendar_href)
            .await
            .ok()
            .flatten();
        if let Some((_, expires_at)) = existing {
            if expires_at > renew_threshold {
                return;
            }
        }

        let (p256dh, auth) = match derive_push_keys(&self.encryption_key, account_id, calendar_href) {
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

        match client
            .register_push(calendar_href, &callback_url, &p256dh, &auth)
            .await
        {
            Ok(Some(expires_at)) => {
                if let Err(e) = db::upsert_push_subscription(
                    &self.db,
                    account_id,
                    calendar_href,
                    &token,
                    "web-push",
                    expires_at,
                )
                .await
                {
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
