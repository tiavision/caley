use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

use crate::caldav::CalDavClient;
use crate::db;
use crate::sse::{SseBroadcaster, SseMessage};

const SYNC_INTERVAL_SECS: u64 = 300;

pub enum SyncTrigger {
    Full,
    Incremental,
}

pub struct SyncManager {
    db: Arc<db::Db>,
    encryption_key: [u8; 32],
    broadcaster: Arc<SseBroadcaster>,
    tasks: Mutex<HashMap<String, mpsc::UnboundedSender<SyncTrigger>>>,
}

impl SyncManager {
    pub fn new(
        db: Arc<db::Db>,
        encryption_key: [u8; 32],
        broadcaster: Arc<SseBroadcaster>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            encryption_key,
            broadcaster,
            tasks: Mutex::new(HashMap::new()),
        })
    }

    /// Ensure a background sync task is running for this user.
    /// Safe to call on every authenticated request — no-ops if already running.
    pub fn ensure_task_for(self: &Arc<Self>, user_id: &str) {
        let mut tasks = self.tasks.lock().unwrap();
        if tasks.contains_key(user_id) {
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tasks.insert(user_id.to_string(), tx);
        drop(tasks);

        let mgr = Arc::clone(self);
        let uid = user_id.to_string();
        tokio::spawn(async move {
            mgr.run_user_loop(uid, rx).await;
        });
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
                _ = sleep(Duration::from_secs(SYNC_INTERVAL_SECS)) => {
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
                        None => break,
                    }
                }
            }
        }

        // Remove from task map so ensure_task_for can restart it
        let mut tasks = self.tasks.lock().unwrap();
        tasks.remove(&user_id);
    }

    async fn sync_user(&self, user_id: &str, full: bool) -> anyhow::Result<()> {
        let settings = {
            let db = self.db.lock().unwrap();
            db::get_settings(&db, user_id, &self.encryption_key)?
        };
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
            let db = self.db.lock().unwrap();
            db::get_cached_calendars(&db, user_id)?.is_empty()
        } else {
            false
        };
        let do_full = full || cached_empty;

        // On full sync, rediscover the calendar list from CalDAV
        let calendars: Vec<db::CachedCalendar> = if do_full {
            let fresh = client.calendars().await?;
            {
                let db = self.db.lock().unwrap();
                for cal in &fresh {
                    db::upsert_cached_calendar(
                        &db,
                        user_id,
                        &cal.href,
                        &cal.name,
                        cal.color.as_deref(),
                    )?;
                }
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
            let db = self.db.lock().unwrap();
            db::get_cached_calendars(&db, user_id)?
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
        let (stored_ctag, stored_token) = {
            let db = self.db.lock().unwrap();
            db::get_calendar_sync_state(&db, user_id, calendar_href)?
        };

        // ── Try RFC 6578 incremental sync ───────────────────────────────────────
        if let Some(ref token) = stored_token {
            match client.sync_collection(calendar_href, Some(token)).await {
                Ok(result) => {
                    let any_change =
                        !result.changed.is_empty() || !result.deleted.is_empty();
                    {
                        let db = self.db.lock().unwrap();
                        for (href, etag, raw_ical) in result.changed {
                            db::upsert_cached_event_raw(
                                &db,
                                user_id,
                                calendar_href,
                                &href,
                                &etag,
                                &raw_ical,
                            )?;
                        }
                        for href in result.deleted {
                            db::delete_event_by_href(&db, user_id, calendar_href, &href)?;
                        }
                        db::update_calendar_sync_state(
                            &db,
                            user_id,
                            calendar_href,
                            stored_ctag.as_deref(),
                            Some(&result.new_token),
                        )?;
                    }
                    if any_change {
                        self.broadcaster.send(user_id, SseMessage::EventsRefetch);
                    }
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
            {
                let db = self.db.lock().unwrap();
                db::clear_calendar_events(&db, user_id, calendar_href)?;
                for (href, etag, raw_ical) in all_raw {
                    db::upsert_cached_event_raw(
                        &db,
                        user_id,
                        calendar_href,
                        &href,
                        &etag,
                        &raw_ical,
                    )?;
                }
                db::update_calendar_sync_state(
                    &db,
                    user_id,
                    calendar_href,
                    new_ctag.as_deref(),
                    new_token.as_deref(),
                )?;
            }
            self.broadcaster.send(user_id, SseMessage::EventsRefetch);
        }

        Ok(())
    }
}
