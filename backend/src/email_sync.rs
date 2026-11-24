use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_imap::extensions::idle::IdleResponse;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

use crate::email_db;
use crate::imap_client;
use crate::search::SearchMode;
use crate::sse::{SseBroadcaster, SseMessage};

pub enum EmailSyncTrigger {
    #[allow(dead_code)]
    Full,
    Mailbox(String),
}

pub struct EmailSyncManager {
    db: sqlx::SqlitePool,
    encryption_key: [u8; 32],
    broadcaster: Arc<SseBroadcaster>,
    sync_interval: Duration,
    search_mode: SearchMode,
    data_dir: PathBuf,
    /// keyed by account_id
    tasks: Mutex<HashMap<String, mpsc::UnboundedSender<EmailSyncTrigger>>>,
    /// persistent IDLE connections, one per account — keyed by account_id
    idle_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    demo_mode: bool,
}

impl EmailSyncManager {
    pub fn new(
        db: sqlx::SqlitePool,
        encryption_key: [u8; 32],
        broadcaster: Arc<SseBroadcaster>,
        sync_interval_mins: u64,
        search_mode: SearchMode,
        data_dir: PathBuf,
        demo_mode: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            encryption_key,
            broadcaster,
            sync_interval: Duration::from_secs(sync_interval_mins.saturating_mul(60)),
            search_mode,
            data_dir,
            tasks: Mutex::new(HashMap::new()),
            idle_tasks: Mutex::new(HashMap::new()),
            demo_mode,
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

    pub async fn stop_all_for(&self, user_id: &str) {
        let account_ids = match email_db::get_all_account_ids(&self.db, user_id).await {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("[email_sync] failed to list accounts for {user_id}: {e}");
                return;
            }
        };

        for id in account_ids {
            self.stop_idle_for(&id);
            self.stop_task_for(&id);
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

    async fn load_session(
        &self,
        account_id: &str,
    ) -> anyhow::Result<(imap_client::ImapSession, String)> {
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
        if self.demo_mode {
            return Ok(());
        }
        let (mut session, user_id) = self.load_session(account_id).await?;

        // Discover + cache mailboxes
        let server_mailboxes = imap_client::list_mailboxes(&mut session).await?;
        let mut changed = false;
        for (name, delim, subscribed) in &server_mailboxes {
            email_db::upsert_mailbox(
                &self.db,
                account_id,
                name,
                delim.as_deref(),
                None,
                *subscribed,
            )
            .await?;
        }
        if !server_mailboxes.is_empty() {
            changed = true;
        }
        if changed {
            self.broadcaster.send(
                &user_id,
                SseMessage::MailboxesChanged {
                    account_id: account_id.to_owned(),
                },
            );
        }

        // Sync each subscribed mailbox
        for (name, _, subscribed) in &server_mailboxes {
            if !subscribed {
                continue;
            }
            self.broadcaster.send(
                &user_id,
                SseMessage::SyncStarted {
                    account_id: account_id.to_owned(),
                    mailbox: name.to_owned(),
                },
            );
            let result = self
                .sync_mailbox_session(&mut session, account_id, &user_id, name)
                .await;
            self.broadcaster.send(
                &user_id,
                SseMessage::SyncCompleted {
                    account_id: account_id.to_owned(),
                    mailbox: name.to_owned(),
                },
            );
            if let Err(e) = result {
                eprintln!("[email_sync] mailbox {name} sync error: {e}");
            }
        }

        let _ = session.logout().await;
        Ok(())
    }

    async fn sync_mailbox(&self, account_id: &str, mailbox: &str) -> anyhow::Result<()> {
        if self.demo_mode {
            return Ok(());
        }
        let (mut session, user_id) = self.load_session(account_id).await?;
        self.broadcaster.send(
            &user_id,
            SseMessage::SyncStarted {
                account_id: account_id.to_owned(),
                mailbox: mailbox.to_owned(),
            },
        );
        let result = self
            .sync_mailbox_session(&mut session, account_id, &user_id, mailbox)
            .await;
        self.broadcaster.send(
            &user_id,
            SseMessage::SyncCompleted {
                account_id: account_id.to_owned(),
                mailbox: mailbox.to_owned(),
            },
        );
        let _ = session.logout().await;
        result
    }

    // ── IMAP IDLE ─────────────────────────────────────────────────────────────

    async fn run_idle_loop(self: Arc<Self>, account_id: String) {
        if self.demo_mode {
            return;
        }
        let mut backoff = Duration::from_secs(5);
        const MAX_BACKOFF: Duration = Duration::from_secs(300);
        loop {
            match self.idle_once(&account_id).await {
                Ok(true) => {
                    backoff = Duration::from_secs(5);
                }
                Ok(false) => {
                    eprintln!(
                        "[idle] {account_id}: server does not support IDLE, falling back to polling"
                    );
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

        let caps = session
            .capabilities()
            .await
            .map_err(|e| anyhow::anyhow!("CAPABILITY failed: {e}"))?;
        if !caps.has_str("IDLE") {
            return Ok(false);
        }

        session
            .select("INBOX")
            .await
            .map_err(|e| anyhow::anyhow!("SELECT INBOX failed: {e}"))?;

        let mut handle = session.idle();
        handle
            .init()
            .await
            .map_err(|e| anyhow::anyhow!("IDLE init failed: {e}"))?;

        loop {
            // RFC 2177: re-issue IDLE at least every 29 minutes. We use 25 min to be safe.
            let (wait_fut, _stop_source) = handle.wait_with_timeout(Duration::from_secs(25 * 60));
            let resp = wait_fut
                .await
                .map_err(|e| anyhow::anyhow!("IDLE wait error: {e}"))?;

            match resp {
                IdleResponse::NewData(_) => {
                    let session = handle
                        .done()
                        .await
                        .map_err(|e| anyhow::anyhow!("IDLE done failed: {e}"))?;
                    self.trigger(account_id, EmailSyncTrigger::Mailbox("INBOX".to_owned()));
                    // Brief pause so the sync task can consume the trigger before we re-IDLE
                    sleep(Duration::from_millis(500)).await;
                    handle = session.idle();
                    handle
                        .init()
                        .await
                        .map_err(|e| anyhow::anyhow!("IDLE re-init failed: {e}"))?;
                }
                IdleResponse::Timeout => {
                    // Keep the connection alive by sending DONE and re-issuing IDLE
                    let session = handle
                        .done()
                        .await
                        .map_err(|e| anyhow::anyhow!("IDLE done (timeout) failed: {e}"))?;
                    handle = session.idle();
                    handle
                        .init()
                        .await
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
        let cached_uidvalidity =
            email_db::get_mailbox_uidvalidity(&self.db, account_id, mailbox).await?;
        let notify_new_inbox_mail =
            mailbox.eq_ignore_ascii_case("INBOX") && cached_uidvalidity == Some(uidvalidity);

        // UIDVALIDITY changed → full re-fetch
        if cached_uidvalidity.map_or(false, |v| v != uidvalidity) {
            email_db::delete_attachments_for_mailbox(&self.db, account_id, mailbox).await?;
            email_db::delete_messages_for_mailbox(&self.db, account_id, mailbox).await?;
            // Delete the mailbox's entire .eml directory
            let mailbox_dir = self
                .data_dir
                .join("messages")
                .join(account_id)
                .join(sha2_hex(mailbox.to_owned()));
            let _ = tokio::fs::remove_dir_all(&mailbox_dir).await;
            self.broadcaster.send(
                user_id,
                SseMessage::EmailsRefetch {
                    account_id: account_id.to_owned(),
                    mailbox: mailbox.to_owned(),
                },
            );
        }

        email_db::upsert_mailbox(&self.db, account_id, mailbox, None, Some(uidvalidity), true)
            .await?;

        let server_uids: HashSet<u32> = imap_client::fetch_all_uids(session)
            .await?
            .into_iter()
            .collect();
        let cached_uids: HashSet<u32> = email_db::get_cached_uids(&self.db, account_id, mailbox)
            .await?
            .into_iter()
            .collect();

        // Deleted on server
        let deleted: Vec<u32> = cached_uids.difference(&server_uids).copied().collect();
        if !deleted.is_empty() {
            for &uid in &deleted {
                let _ = tokio::fs::remove_file(eml_path(&self.data_dir, account_id, mailbox, uid))
                    .await;
            }
            email_db::delete_attachments_by_uids(&self.db, account_id, mailbox, &deleted).await?;
            email_db::delete_messages_by_uids(&self.db, account_id, mailbox, &deleted).await?;
            for uid in &deleted {
                self.broadcaster.send(
                    user_id,
                    SseMessage::EmailRemoved {
                        account_id: account_id.to_owned(),
                        uid: *uid,
                        mailbox: mailbox.to_owned(),
                    },
                );
            }
        }

        let surviving_cached_uids: Vec<u32> =
            server_uids.intersection(&cached_uids).copied().collect();
        if !surviving_cached_uids.is_empty() {
            let cached_flags =
                email_db::get_cached_message_flags(&self.db, account_id, mailbox).await?;
            for chunk in surviving_cached_uids.chunks(200) {
                let remote_flags = imap_client::fetch_message_flags(session, chunk).await?;
                for (uid, flags) in remote_flags {
                    let flags = normalize_flags(flags);
                    let cached = cached_flags.get(&uid).and_then(|f| f.as_deref());
                    if flags_match(cached, &flags) {
                        continue;
                    }

                    let flags_json = serde_json::to_string(&flags)?;
                    email_db::update_message_flags(&self.db, account_id, mailbox, uid, &flags_json)
                        .await?;
                    self.broadcaster.send(
                        user_id,
                        SseMessage::EmailUpdated {
                            account_id: account_id.to_owned(),
                            uid,
                            mailbox: mailbox.to_owned(),
                            flags,
                        },
                    );
                }
            }
        }

        // New on server — fetch headers
        let new_uids: Vec<u32> = server_uids.difference(&cached_uids).copied().collect();
        if !new_uids.is_empty() {
            let headers = imap_client::fetch_headers(session, &new_uids).await?;
            for h in &headers {
                let subject_idx = h
                    .subject
                    .as_deref()
                    .map(|s| self.search_mode.normalize_text(s));
                let from_idx = h
                    .from_addr
                    .as_deref()
                    .map(|s| self.search_mode.normalize_text(s));
                let to_idx = h
                    .to_addrs
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
                    .map(|addrs| self.search_mode.normalize_text(&addrs.join(" ")));
                email_db::upsert_message_header(
                    &self.db,
                    account_id,
                    mailbox,
                    h,
                    subject_idx.as_deref(),
                    from_idx.as_deref(),
                    to_idx.as_deref(),
                )
                .await?;
                email_db::replace_message_addresses(
                    &self.db,
                    account_id,
                    mailbox,
                    h.uid,
                    &h.from_emails,
                    &h.to_emails,
                    &h.cc_emails,
                )
                .await?;
                if notify_new_inbox_mail
                    && !h.flags.as_deref().unwrap_or_default().contains("\\Seen")
                {
                    self.broadcaster.send(
                        user_id,
                        SseMessage::NewEmail {
                            account_id: account_id.to_owned(),
                            uid: h.uid as u32,
                            mailbox: mailbox.to_owned(),
                            from_addr: h.from_addr.clone(),
                            subject: h.subject.clone(),
                        },
                    );
                }
            }
            if !headers.is_empty() {
                self.broadcaster.send(
                    user_id,
                    SseMessage::EmailsRefetch {
                        account_id: account_id.to_owned(),
                        mailbox: mailbox.to_owned(),
                    },
                );
            }
        }

        // Fetch bodies + attachments eagerly for any cached messages missing them
        let missing_body_uids =
            email_db::get_cached_uids_missing_body(&self.db, account_id, mailbox).await?;
        let uids_to_fetch: Vec<u32> = new_uids
            .into_iter()
            .chain(missing_body_uids)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        if !uids_to_fetch.is_empty() {
            for chunk in uids_to_fetch.chunks(50) {
                let bodies = match imap_client::fetch_bodies(session, chunk).await {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("[email_sync] fetch_bodies error for {mailbox}: {e}");
                        continue;
                    }
                };

                // Write .eml files to disk first (outside the DB transaction) so the
                // SQLite write lock is held only for the fast DB writes, not file I/O.
                let mut prepared: Vec<PreparedMessage> = Vec::with_capacity(bodies.len());
                for (uid, raw_bytes, render_body, has_attach, attachments) in bodies {
                    let is_encrypted = imap_client::detect_pgp_encrypted(&raw_bytes);
                    // For encrypted messages, exclude body from FTS index.
                    let body_idx = if is_encrypted {
                        String::new()
                    } else {
                        self.search_mode.normalize_text(&render_body.search_text)
                    };

                    let path = eml_path(&self.data_dir, account_id, mailbox, uid);
                    if let Some(dir) = path.parent() {
                        tokio::fs::create_dir_all(dir).await?;
                    }
                    let compressed = zstd::encode_all(raw_bytes.as_slice(), 6)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                    tokio::fs::write(&path, compressed).await?;

                    let att_params = attachments
                        .into_iter()
                        .map(|att| PreparedAttachment {
                            filename: att.filename.unwrap_or_else(|| "untitled".to_owned()),
                            content_type: att.content_type,
                            size: att.size,
                            part_index: att.part_index,
                            content_id: att.content_id,
                        })
                        .collect();

                    prepared.push(PreparedMessage {
                        uid,
                        has_attach,
                        body_idx,
                        is_encrypted,
                        is_signed: render_body.is_signed,
                        attachments: att_params,
                    });
                }

                // Commit all DB writes for the chunk in a single transaction (one fsync
                // instead of dozens), then yield so concurrent calendar syncs can write.
                let mut tx = self.db.begin().await?;
                for m in &prepared {
                    email_db::update_message_body(
                        &mut *tx,
                        account_id,
                        mailbox,
                        m.uid,
                        &m.body_idx,
                        m.has_attach,
                        m.is_encrypted,
                    )
                    .await?;
                    email_db::upsert_message_render_body(
                        &mut *tx,
                        account_id,
                        mailbox,
                        m.uid,
                        m.is_signed,
                    )
                    .await?;
                    for att in &m.attachments {
                        email_db::insert_attachment(
                            &mut *tx,
                            account_id,
                            mailbox,
                            m.uid,
                            &att.filename,
                            &att.content_type,
                            att.size,
                            att.part_index as i64,
                            att.content_id.as_deref(),
                        )
                        .await?;
                    }
                }
                tx.commit().await?;
                tokio::task::yield_now().await;
            }
        }

        Ok(())
    }
}

struct PreparedMessage {
    uid: u32,
    has_attach: bool,
    body_idx: String,
    is_encrypted: bool,
    is_signed: bool,
    attachments: Vec<PreparedAttachment>,
}

struct PreparedAttachment {
    filename: String,
    content_type: String,
    size: i64,
    part_index: usize,
    content_id: Option<String>,
}

fn normalize_flags(mut flags: Vec<String>) -> Vec<String> {
    flags.retain(|f| !f.is_empty());
    flags.sort();
    flags.dedup();
    flags
}

fn flags_match(cached_json: Option<&str>, remote_flags: &[String]) -> bool {
    let cached_flags = cached_json
        .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
        .map(normalize_flags)
        .unwrap_or_default();
    cached_flags == remote_flags
}

fn sha2_hex(input: String) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Deterministic path to the compressed `.eml.zst` file for a given message.
pub fn eml_path(
    data_dir: &std::path::Path,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> std::path::PathBuf {
    data_dir
        .join("messages")
        .join(account_id)
        .join(sha2_hex(mailbox.to_owned()))
        .join(format!("{uid}.eml.zst"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_comparison_ignores_order_and_duplicates() {
        let remote = normalize_flags(vec![
            "\\Flagged".to_owned(),
            "\\Seen".to_owned(),
            "\\Seen".to_owned(),
        ]);

        assert!(flags_match(Some(r#"["\\Seen","\\Flagged"]"#), &remote));
        assert!(!flags_match(Some(r#"["\\Seen"]"#), &remote));
    }
}
