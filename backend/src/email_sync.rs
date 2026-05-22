use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_imap::extensions::idle::IdleResponse;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

use crate::email_db;
use crate::imap_client;
use crate::search::SearchMode;
use crate::sse::{SseBroadcaster, SseMessage};

pub enum EmailSyncTrigger {
    Full,
    Mailbox(String),
}

pub struct EmailSyncManager {
    db: sqlx::SqlitePool,
    encryption_key: [u8; 32],
    broadcaster: Arc<SseBroadcaster>,
    sync_interval: Duration,
    search_mode: SearchMode,
    /// keyed by account_id
    tasks: Mutex<HashMap<String, mpsc::UnboundedSender<EmailSyncTrigger>>>,
    /// persistent IDLE connections, one per account — keyed by account_id
    idle_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl EmailSyncManager {
    pub fn new(
        db: sqlx::SqlitePool,
        encryption_key: [u8; 32],
        broadcaster: Arc<SseBroadcaster>,
        sync_interval_mins: u64,
        search_mode: SearchMode,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            encryption_key,
            broadcaster,
            sync_interval: Duration::from_secs(sync_interval_mins.saturating_mul(60)),
            search_mode,
            tasks: Mutex::new(HashMap::new()),
            idle_tasks: Mutex::new(HashMap::new()),
        })
    }

    /// Start sync tasks for all accounts belonging to this user (called on login / SSE connect).
    pub async fn ensure_tasks_for_user(self: &Arc<Self>, user_id: &str) {
        let account_ids = match email_db::get_all_account_ids(&self.db, user_id).await {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("[email_sync] failed to list accounts for {user_id}: {e}");
                return;
            }
        };
        for id in account_ids {
            self.ensure_task_for(&id);
        }
    }

    pub fn ensure_task_for(self: &Arc<Self>, account_id: &str) {
        let mut tasks = self.tasks.lock().unwrap();
        if tasks.contains_key(account_id) {
            drop(tasks);
            self.ensure_idle_for(account_id);
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tasks.insert(account_id.to_owned(), tx);
        drop(tasks);

        let mgr = Arc::clone(self);
        let aid = account_id.to_owned();
        tokio::spawn(async move {
            mgr.run_account_loop(aid, rx).await;
        });

        self.ensure_idle_for(account_id);
    }

    /// Start an IMAP IDLE watcher for this account if one isn't already running.
    pub fn ensure_idle_for(self: &Arc<Self>, account_id: &str) {
        let mut idle_tasks = self.idle_tasks.lock().unwrap();
        if let Some(h) = idle_tasks.get(account_id) {
            if !h.is_finished() {
                return;
            }
        }
        let mgr = Arc::clone(self);
        let aid = account_id.to_owned();
        let handle = tokio::spawn(async move {
            mgr.run_idle_loop(aid).await;
        });
        idle_tasks.insert(account_id.to_owned(), handle);
    }

    /// Abort the IDLE watcher for this account (call on account deletion or credential update).
    pub fn stop_idle_for(&self, account_id: &str) {
        if let Some(h) = self.idle_tasks.lock().unwrap().remove(account_id) {
            h.abort();
        }
    }

    /// Drop the sync task sender for this account, causing the sync loop to exit.
    pub fn stop_task_for(&self, account_id: &str) {
        self.tasks.lock().unwrap().remove(account_id);
    }

    pub fn trigger(&self, account_id: &str, trigger: EmailSyncTrigger) {
        let tasks = self.tasks.lock().unwrap();
        if let Some(tx) = tasks.get(account_id) {
            let _ = tx.send(trigger);
        }
    }

    async fn run_account_loop(
        self: Arc<Self>,
        account_id: String,
        mut rx: mpsc::UnboundedReceiver<EmailSyncTrigger>,
    ) {
        // Initial sync
        if let Err(e) = self.sync_account(&account_id).await {
            eprintln!("[email_sync] initial sync failed for account {account_id}: {e}");
        }

        loop {
            tokio::select! {
                _ = sleep(self.sync_interval) => {
                    if let Err(e) = self.sync_account(&account_id).await {
                        eprintln!("[email_sync] periodic sync failed for account {account_id}: {e}");
                    }
                }
                msg = rx.recv() => {
                    match msg {
                        Some(EmailSyncTrigger::Full) => {
                            if let Err(e) = self.sync_account(&account_id).await {
                                eprintln!("[email_sync] full sync failed for account {account_id}: {e}");
                            }
                        }
                        Some(EmailSyncTrigger::Mailbox(mb)) => {
                            if let Err(e) = self.sync_mailbox(&account_id, &mb).await {
                                eprintln!("[email_sync] mailbox sync failed for {mb}: {e}");
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        let mut tasks = self.tasks.lock().unwrap();
        tasks.remove(&account_id);
    }

    async fn load_session(&self, account_id: &str) -> anyhow::Result<(imap_client::ImapSession, String)> {
        // We need user_id to look up the account; get it from DB
        let row = sqlx::query("SELECT user_id FROM imap_accounts WHERE account_id = ?")
            .bind(account_id)
            .fetch_optional(&self.db)
            .await?;
        let user_id: String = match row {
            Some(r) => sqlx::Row::get(&r, 0),
            None => return Err(anyhow::anyhow!("account {account_id} not found")),
        };

        let account = email_db::get_account(&self.db, account_id, &user_id, &self.encryption_key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("account {account_id} not found for user"))?;

        let session = imap_client::connect(
            &account.host,
            account.port as u16,
            &account.username,
            &account.password,
        )
        .await?;
        Ok((session, user_id))
    }

    async fn sync_account(&self, account_id: &str) -> anyhow::Result<()> {
        let (mut session, user_id) = self.load_session(account_id).await?;

        // Discover + cache mailboxes
        let server_mailboxes = imap_client::list_mailboxes(&mut session).await?;
        let mut changed = false;
        for (name, delim, subscribed) in &server_mailboxes {
            email_db::upsert_mailbox(&self.db, account_id, name, delim.as_deref(), None, *subscribed).await?;
        }
        if !server_mailboxes.is_empty() {
            changed = true;
        }
        if changed {
            self.broadcaster.send(&user_id, SseMessage::MailboxesChanged { account_id: account_id.to_owned() });
        }

        // Sync each subscribed mailbox
        for (name, _, subscribed) in &server_mailboxes {
            if !subscribed {
                continue;
            }
            if let Err(e) = self.sync_mailbox_session(&mut session, account_id, &user_id, name).await {
                eprintln!("[email_sync] mailbox {name} sync error: {e}");
            }
        }

        let _ = session.logout().await;
        Ok(())
    }

    async fn sync_mailbox(&self, account_id: &str, mailbox: &str) -> anyhow::Result<()> {
        let (mut session, user_id) = self.load_session(account_id).await?;
        self.sync_mailbox_session(&mut session, account_id, &user_id, mailbox).await?;
        let _ = session.logout().await;
        Ok(())
    }

    // ── IMAP IDLE ─────────────────────────────────────────────────────────────

    async fn run_idle_loop(self: Arc<Self>, account_id: String) {
        let mut backoff = Duration::from_secs(5);
        const MAX_BACKOFF: Duration = Duration::from_secs(300);
        loop {
            match self.idle_once(&account_id).await {
                Ok(true) => {
                    backoff = Duration::from_secs(5);
                }
                Ok(false) => {
                    eprintln!("[idle] {account_id}: server does not support IDLE, falling back to polling");
                    break;
                }
                Err(e) => {
                    eprintln!("[idle] {account_id}: error ({e}), retrying in {backoff:?}");
                    sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
        self.idle_tasks.lock().unwrap().remove(&account_id);
    }

    /// Opens a dedicated IMAP connection, enters IDLE on INBOX, and triggers
    /// a mailbox sync whenever the server signals a change.
    ///
    /// Returns `Ok(true)` on a clean disconnect/reconnect cycle, `Ok(false)` if
    /// the server doesn't support IDLE, and `Err` on any other failure.
    async fn idle_once(&self, account_id: &str) -> anyhow::Result<bool> {
        let (mut session, _user_id) = self.load_session(account_id).await?;

        let caps = session.capabilities().await
            .map_err(|e| anyhow::anyhow!("CAPABILITY failed: {e}"))?;
        if !caps.has_str("IDLE") {
            return Ok(false);
        }

        session.select("INBOX").await
            .map_err(|e| anyhow::anyhow!("SELECT INBOX failed: {e}"))?;

        let mut handle = session.idle();
        handle.init().await
            .map_err(|e| anyhow::anyhow!("IDLE init failed: {e}"))?;

        loop {
            // RFC 2177: re-issue IDLE at least every 29 minutes. We use 25 min to be safe.
            let (wait_fut, _stop_source) = handle.wait_with_timeout(Duration::from_secs(25 * 60));
            let resp = wait_fut.await
                .map_err(|e| anyhow::anyhow!("IDLE wait error: {e}"))?;

            match resp {
                IdleResponse::NewData(_) => {
                    let session = handle.done().await
                        .map_err(|e| anyhow::anyhow!("IDLE done failed: {e}"))?;
                    self.trigger(account_id, EmailSyncTrigger::Mailbox("INBOX".to_owned()));
                    // Brief pause so the sync task can consume the trigger before we re-IDLE
                    sleep(Duration::from_millis(500)).await;
                    handle = session.idle();
                    handle.init().await
                        .map_err(|e| anyhow::anyhow!("IDLE re-init failed: {e}"))?;
                }
                IdleResponse::Timeout => {
                    // Keep the connection alive by sending DONE and re-issuing IDLE
                    let session = handle.done().await
                        .map_err(|e| anyhow::anyhow!("IDLE done (timeout) failed: {e}"))?;
                    handle = session.idle();
                    handle.init().await
                        .map_err(|e| anyhow::anyhow!("IDLE re-init (timeout) failed: {e}"))?;
                }
                IdleResponse::ManualInterrupt => {
                    let _ = handle.done().await;
                    return Ok(true);
                }
            }
        }
    }

    async fn sync_mailbox_session(
        &self,
        session: &mut imap_client::ImapSession,
        account_id: &str,
        user_id: &str,
        mailbox: &str,
    ) -> anyhow::Result<()> {
        let (uidvalidity, _exists) = imap_client::select_mailbox(session, mailbox).await?;
        let cached_uidvalidity = email_db::get_mailbox_uidvalidity(&self.db, account_id, mailbox).await?;

        // UIDVALIDITY changed → full re-fetch
        if cached_uidvalidity.map_or(false, |v| v != uidvalidity) {
            email_db::delete_messages_for_mailbox(&self.db, account_id, mailbox).await?;
            self.broadcaster.send(user_id, SseMessage::EmailsRefetch {
                account_id: account_id.to_owned(),
                mailbox: mailbox.to_owned(),
            });
        }

        email_db::upsert_mailbox(&self.db, account_id, mailbox, None, Some(uidvalidity), true).await?;

        let server_uids: HashSet<u32> = imap_client::fetch_all_uids(session).await?.into_iter().collect();
        let cached_uids: HashSet<u32> = email_db::get_cached_uids(&self.db, account_id, mailbox)
            .await?
            .into_iter()
            .collect();

        // Deleted on server
        let deleted: Vec<u32> = cached_uids.difference(&server_uids).copied().collect();
        if !deleted.is_empty() {
            email_db::delete_messages_by_uids(&self.db, account_id, mailbox, &deleted).await?;
            for uid in &deleted {
                self.broadcaster.send(user_id, SseMessage::EmailRemoved {
                    account_id: account_id.to_owned(),
                    uid: *uid,
                    mailbox: mailbox.to_owned(),
                });
            }
        }

        // New on server
        let new_uids: Vec<u32> = server_uids.difference(&cached_uids).copied().collect();
        if !new_uids.is_empty() {
            let headers = imap_client::fetch_headers(session, &new_uids, &self.search_mode).await?;
            for h in &headers {
                let subject_idx = h.subject.as_deref().map(|s| self.search_mode.normalize_text(s));
                let from_idx = h.from_addr.as_deref().map(|s| self.search_mode.normalize_text(s));
                email_db::upsert_message_header(
                    &self.db, account_id, mailbox, h,
                    subject_idx.as_deref(),
                    from_idx.as_deref(),
                ).await?;
            }
            if headers.len() == 1 {
                let h = &headers[0];
                self.broadcaster.send(user_id, SseMessage::EmailUpdated {
                    account_id: account_id.to_owned(),
                    uid: h.uid as u32,
                    mailbox: mailbox.to_owned(),
                    flags: h.flags.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default(),
                });
            } else if !headers.is_empty() {
                self.broadcaster.send(user_id, SseMessage::EmailsRefetch {
                    account_id: account_id.to_owned(),
                    mailbox: mailbox.to_owned(),
                });
            }
        }

        Ok(())
    }
}
