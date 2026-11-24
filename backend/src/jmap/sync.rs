//! A JMAP account's mail into the cache: the same rows, `.eml` files and server events an IMAP account's sync makes,
//! so the API and the frontend do not tell the two apart.
//!
//! The cache knows a message as (account, mailbox name, UID). A JMAP email has an id of its own and may be in several
//! mailboxes, so each mailbox it is in gives it a UID there (`jmap_messages`, UIDs from `jmap_mailboxes.next_uid`,
//! never reused): as in IMAP, a move is a removal from one mailbox and a new message in the other.
//!
//! The first sync lists every email (Email/query) and keeps the Email state; the next ones ask for the changes since
//! that state (Email/changes), which push (EventSource) asks for as soon as the server has any. A state the server can
//! no longer calculate changes from (`cannotCalculateChanges`) starts over. Mailboxes are read whole every time
//! (there are few); a renamed one is dropped from the cache and its emails listed again under the new name.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tokio::time::{Duration, sleep};

use super::{Auth, Client, Email};
use crate::email_db::{self, ImapAccountRow, jmap_db};
use crate::email_sync::{EmailSyncManager, EmailSyncTrigger, eml_path, mailbox_dir};
use crate::imap_client;
use crate::sse::SseMessage;

/// Blobs downloaded at once while bodies are synced.
const DOWNLOADS_AT_ONCE: usize = 4;

impl EmailSyncManager {
    /// How a JMAP account signs in now: an OIDC account takes the user's current access token (renewed if needed).
    pub(crate) async fn jmap_auth(&self, account: &ImapAccountRow, user_id: &str) -> Result<Auth> {
        Ok(match account.auth.as_str() {
            "oidc" => Auth::Bearer(self.oidc_tokens.access_token(user_id).await?),
            "bearer" => Auth::Bearer(account.password.clone()),
            _ => Auth::Basic {
                username: account.username.clone(),
                password: account.password.clone(),
            },
        })
    }

    /// Connects to a JMAP account.
    pub(crate) async fn jmap_client(
        &self,
        account: &ImapAccountRow,
        user_id: &str,
    ) -> Result<Client> {
        let url = account
            .jmap_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("JMAP account {} has no URL", account.account_id))?;
        let auth = self.jmap_auth(account, user_id).await?;
        Client::connect(&self.http, url, auth).await
    }

    /// Syncs a JMAP account. With `mailbox` (a sync the page asked for) its start and end are reported as the
    /// mailbox's.
    pub(crate) async fn jmap_sync(
        &self,
        account: &ImapAccountRow,
        user_id: &str,
        mailbox: Option<&str>,
    ) -> Result<()> {
        let account_id = account.account_id.as_str();
        if let Some(mb) = mailbox {
            self.broadcaster.send(
                user_id,
                SseMessage::SyncStarted {
                    account_id: account_id.to_owned(),
                    mailbox: mb.to_owned(),
                },
            );
        }
        let lock = Arc::clone(
            self.jmap_locks
                .lock()
                .unwrap()
                .entry(account_id.to_owned())
                .or_default(),
        );
        let result = async {
            let _one_at_a_time = lock.lock().await;
            let client = self.jmap_client(account, user_id).await?;
            self.jmap_sync_with(&client, account_id, user_id).await
        }
        .await;
        if let Some(mb) = mailbox {
            self.broadcaster.send(
                user_id,
                SseMessage::SyncCompleted {
                    account_id: account_id.to_owned(),
                    mailbox: mb.to_owned(),
                },
            );
        }
        result
    }

    /// The sync itself, over a connected client.
    pub(crate) async fn jmap_sync_with(
        &self,
        client: &Client,
        account_id: &str,
        user_id: &str,
    ) -> Result<()> {
        let (names, requery) = self
            .jmap_sync_mailboxes(client, account_id, user_id)
            .await?;

        let mut added: HashSet<String> = HashSet::new();
        let since = jmap_db::jmap_state(&self.db, account_id, "Email").await?;
        let mut full = since.is_none();
        if let Some(since) = since {
            match client.changes("Email", &since).await {
                Ok(changes) => {
                    let ids: Vec<String> = changes
                        .created
                        .iter()
                        .chain(&changes.updated)
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    let (emails, _) = client.emails(&ids).await?;
                    let got: HashSet<&str> = emails.iter().map(|e| e.id.as_str()).collect();
                    for email in &emails {
                        self.jmap_reconcile(account_id, user_id, email, &names, true, &mut added)
                            .await?;
                    }
                    // destroyed, or created and destroyed again before this sync asked
                    let gone = changes
                        .destroyed
                        .iter()
                        .chain(ids.iter().filter(|id| !got.contains(id.as_str())));
                    for id in gone {
                        self.jmap_forget_email(account_id, user_id, id).await?;
                    }
                    if !changes.new_state.is_empty() {
                        jmap_db::set_jmap_state(&self.db, account_id, "Email", &changes.new_state)
                            .await?;
                    }
                }
                // invalidArguments: a state the server does not know at all (Stalwart's answer to a state of
                // another server, or of its own before its data was reset)
                Err(e)
                    if matches!(
                        super::method_error(&e),
                        Some("cannotCalculateChanges" | "invalidArguments")
                    ) =>
                {
                    eprintln!(
                        "[jmap] {account_id}: the server cannot tell the changes, listing everything"
                    );
                    full = true;
                }
                Err(e) => return Err(e),
            }
        }

        if full {
            // the state before the listing: changes made during it come again with the next sync (harmless)
            let state = client.state("Email").await?;
            let ids = client.query_ids(serde_json::Value::Null).await?;
            let (emails, _) = client.emails(&ids).await?;
            let listed: HashSet<&str> = emails.iter().map(|e| e.id.as_str()).collect();
            for email in &emails {
                self.jmap_reconcile(account_id, user_id, email, &names, false, &mut added)
                    .await?;
            }
            for id in jmap_db::jmap_email_ids(&self.db, account_id).await? {
                if !listed.contains(id.as_str()) {
                    self.jmap_forget_email(account_id, user_id, &id).await?;
                }
            }
            jmap_db::set_jmap_state(&self.db, account_id, "Email", &state).await?;
        } else {
            for mailbox_id in requery {
                let ids = client.query_ids(json!({ "inMailbox": mailbox_id })).await?;
                let (emails, _) = client.emails(&ids).await?;
                for email in &emails {
                    self.jmap_reconcile(account_id, user_id, email, &names, false, &mut added)
                        .await?;
                }
            }
        }

        for mailbox in &added {
            self.broadcaster.send(
                user_id,
                SseMessage::EmailsRefetch {
                    account_id: account_id.to_owned(),
                    mailbox: mailbox.clone(),
                },
            );
        }

        self.jmap_sync_bodies(client, account_id).await
    }

    /// Reads the mailboxes into the cache: their names (id → name) and the ids of the renamed ones, whose emails
    /// are to be listed again.
    async fn jmap_sync_mailboxes(
        &self,
        client: &Client,
        account_id: &str,
        user_id: &str,
    ) -> Result<(HashMap<String, String>, Vec<String>)> {
        let (mailboxes, _) = client.mailboxes().await?;
        let names = super::mailbox_names(&mailboxes);
        let mut requery = Vec::new();

        // gone or renamed first, so that their names are free for the others
        for row in jmap_db::jmap_mailboxes(&self.db, account_id).await? {
            match names.get(&row.mailbox_id) {
                Some(name) if *name == row.name => {}
                renamed => {
                    self.jmap_drop_mailbox(account_id, &row.name).await?;
                    jmap_db::delete_jmap_mailbox(&self.db, account_id, &row.mailbox_id).await?;
                    if renamed.is_some() {
                        requery.push(row.mailbox_id);
                    }
                }
            }
        }
        for mb in &mailboxes {
            let name = &names[&mb.id];
            jmap_db::upsert_jmap_mailbox(&self.db, account_id, &mb.id, name, mb.role.as_deref())
                .await?;
            email_db::upsert_mailbox(
                &self.db,
                account_id,
                name,
                Some("/"),
                None,
                mb.is_subscribed,
            )
            .await?;
        }
        self.broadcaster.send(
            user_id,
            SseMessage::MailboxesChanged {
                account_id: account_id.to_owned(),
            },
        );
        Ok((names, requery))
    }

    /// Makes the cache hold an email where the server has it: removed from the mailboxes it left, added (with a new
    /// UID) to the ones it joined, its flags updated in the others. `notify`: a new unread message in the inbox is
    /// reported with `new_email`.
    async fn jmap_reconcile(
        &self,
        account_id: &str,
        user_id: &str,
        email: &Email,
        names: &HashMap<String, String>,
        notify: bool,
        added: &mut HashSet<String>,
    ) -> Result<()> {
        let want: BTreeSet<&str> = email
            .mailbox_ids
            .iter()
            .filter(|(_, set)| **set)
            .filter_map(|(id, _)| names.get(id).map(String::as_str))
            .collect();
        let have = jmap_db::jmap_memberships(&self.db, account_id, &email.id).await?;
        let flags = email.flags();

        for (mailbox, uid) in &have {
            if !want.contains(mailbox.as_str()) {
                self.jmap_remove_cached(account_id, user_id, mailbox, *uid)
                    .await;
                continue;
            }
            let cached = email_db::get_message(&self.db, account_id, mailbox, *uid)
                .await?
                .and_then(|row| row.flags)
                .and_then(|f| serde_json::from_str::<Vec<String>>(&f).ok())
                .map(normalize)
                .unwrap_or_default();
            if cached != flags {
                let flags_json = serde_json::to_string(&flags)?;
                email_db::update_message_flags(&self.db, account_id, mailbox, *uid, &flags_json)
                    .await?;
                self.broadcaster.send(
                    user_id,
                    SseMessage::EmailUpdated {
                        account_id: account_id.to_owned(),
                        uid: *uid,
                        mailbox: mailbox.clone(),
                        flags: flags.clone(),
                    },
                );
            }
        }

        let have_names: HashSet<&str> = have.iter().map(|(m, _)| m.as_str()).collect();
        let header_bytes = email.header_bytes();
        for mailbox in want.iter().filter(|m| !have_names.contains(*m)) {
            let uid = jmap_db::next_uid(&self.db, account_id, mailbox).await?;
            let h = imap_client::header_from_bytes(uid, &header_bytes, &flags, email.size);
            self.store_header(
                account_id,
                user_id,
                mailbox,
                &h,
                notify && mailbox.eq_ignore_ascii_case("INBOX"),
            )
            .await?;
            jmap_db::insert_jmap_message(
                &self.db,
                account_id,
                mailbox,
                uid,
                &email.id,
                &email.blob_id,
            )
            .await?;
            added.insert((*mailbox).to_owned());
        }
        Ok(())
    }

    /// Removes an email from every mailbox of the cache.
    async fn jmap_forget_email(
        &self,
        account_id: &str,
        user_id: &str,
        email_id: &str,
    ) -> Result<()> {
        for (mailbox, uid) in jmap_db::jmap_memberships(&self.db, account_id, email_id).await? {
            self.jmap_remove_cached(account_id, user_id, &mailbox, uid)
                .await;
        }
        Ok(())
    }

    /// Drops a cached message (its `.eml`, attachments, row and JMAP id); `email_removed` reports it.
    async fn jmap_remove_cached(&self, account_id: &str, user_id: &str, mailbox: &str, uid: u32) {
        let _ = tokio::fs::remove_file(eml_path(&self.data_dir, account_id, mailbox, uid)).await;
        let _ = email_db::delete_attachments_for_message(&self.db, account_id, mailbox, uid).await;
        let _ = email_db::delete_message(&self.db, account_id, mailbox, uid).await;
        self.broadcaster.send(
            user_id,
            SseMessage::EmailRemoved {
                account_id: account_id.to_owned(),
                uid,
                mailbox: mailbox.to_owned(),
            },
        );
    }

    /// Drops a mailbox from the cache: its messages, their files, its row.
    async fn jmap_drop_mailbox(&self, account_id: &str, name: &str) -> Result<()> {
        email_db::delete_attachments_for_mailbox(&self.db, account_id, name).await?;
        email_db::delete_messages_for_mailbox(&self.db, account_id, name).await?;
        let _ = tokio::fs::remove_dir_all(mailbox_dir(&self.data_dir, account_id, name)).await;
        jmap_db::delete_cached_mailbox(&self.db, account_id, name).await?;
        Ok(())
    }

    /// Downloads the sources of the messages that have none yet and caches them as an IMAP body fetch would. An
    /// email in several mailboxes is downloaded once. A failed download is tried again with the next sync.
    async fn jmap_sync_bodies(&self, client: &Client, account_id: &str) -> Result<()> {
        let missing = jmap_db::jmap_messages_missing_body(&self.db, account_id).await?;
        let mut by_blob: Vec<(String, Vec<(String, u32)>)> = Vec::new();
        for (mailbox, uid, blob) in missing {
            match by_blob.last_mut() {
                Some((b, places)) if *b == blob => places.push((mailbox, uid)),
                _ => by_blob.push((blob, vec![(mailbox, uid)])),
            }
        }
        for batch in by_blob.chunks(DOWNLOADS_AT_ONCE) {
            let raws =
                futures_util::future::join_all(batch.iter().map(|(blob, _)| client.download(blob)))
                    .await;
            let downloads = raws.into_iter().zip(batch.iter().map(|(_, places)| places));
            let mut per_mailbox: HashMap<String, Vec<_>> = HashMap::new();
            for (raw, places) in downloads {
                let raw = match raw {
                    Ok(raw) => raw,
                    Err(e) => {
                        eprintln!("[jmap] {account_id}: download failed: {e}");
                        continue;
                    }
                };
                for (mailbox, uid) in places {
                    let (render, has_attach, attachments) =
                        match imap_client::parse_body_bytes(&raw) {
                            Ok(parsed) => parsed,
                            Err(e) => {
                                eprintln!("[jmap] {account_id}: {mailbox} {uid}: {e}");
                                continue;
                            }
                        };
                    per_mailbox.entry(mailbox.clone()).or_default().push((
                        *uid,
                        raw.clone(),
                        render,
                        has_attach,
                        attachments,
                    ));
                }
            }
            for (mailbox, bodies) in per_mailbox {
                self.store_bodies(account_id, &mailbox, bodies).await?;
            }
        }
        Ok(())
    }

    /// Listens to a JMAP account's push stream and syncs the account whenever its mail changes; reconnects after
    /// errors (backing off), returns when the server offers no push (the periodic sync remains).
    pub(crate) async fn run_push_loop(self: &Arc<Self>, account_id: &str) {
        let mut backoff = Duration::from_secs(5);
        const MAX_BACKOFF: Duration = Duration::from_secs(300);
        loop {
            let result = async {
                let (account, user_id) = self.load_account(account_id).await?;
                let client = self.jmap_client(&account, &user_id).await?;
                if client.session.event_source_url.is_none() {
                    return Ok(false);
                }
                let mgr = Arc::clone(self);
                client
                    .listen("Email,Mailbox", 300, |changed| {
                        if changed.contains_key("Email") || changed.contains_key("Mailbox") {
                            mgr.trigger(account_id, EmailSyncTrigger::Full);
                        }
                    })
                    .await?;
                Ok::<_, anyhow::Error>(true)
            }
            .await;
            match result {
                Ok(true) => {
                    backoff = Duration::from_secs(5);
                    sleep(Duration::from_secs(1)).await;
                }
                Ok(false) => {
                    eprintln!("[jmap] {account_id}: the server offers no push, polling");
                    return;
                }
                Err(e) => {
                    eprintln!("[jmap] {account_id}: push: {e}, again in {backoff:?}");
                    sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
    }
}

fn normalize(mut flags: Vec<String>) -> Vec<String> {
    flags.retain(|f| !f.is_empty());
    flags.sort();
    flags.dedup();
    flags
}

#[cfg(test)]
mod tests {
    use crate::jmap::fake::FakeJmap;

    /// Two syncs of one account at once (a restarted task, a sync the page asked for) run one after the other.
    #[tokio::test]
    async fn concurrent_syncs_give_an_email_one_uid_per_mailbox() {
        let fake = FakeJmap::start().await;
        let inbox = fake.mailbox_id("inbox");
        for i in 0..20 {
            fake.deliver(
                &[&inbox],
                &crate::jmap::tests::mail(&format!("m{i}"), "a@example.org"),
                &[],
            );
        }
        let state = crate::jmap::api_tests::app_state().await;
        let (_, id) = crate::jmap::api_tests::save(
            &state,
            crate::email::AccountBody {
                account_id: None,
                display_name: "Fake".into(),
                protocol: Some(crate::email::Protocol::Jmap),
                jmap_url: Some(fake.url.clone()),
                auth: Some(crate::email::AccountAuth::Password),
                host: String::new(),
                port: 0,
                username: "user".into(),
                password: Some("secret".into()),
                use_tls: None,
                smtp_host: None,
                smtp_port: None,
                sender_name: None,
            },
        )
        .await;
        let mgr = &state.email_sync_manager;
        let (account, user) = mgr.load_account(&id).await.unwrap();
        let (a, b) = tokio::join!(
            mgr.jmap_sync(&account, &user, None),
            mgr.jmap_sync(&account, &user, None)
        );
        a.unwrap();
        b.unwrap();
        mgr.stop_task_for(&id);
        mgr.stop_idle_for(&id);
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cached_messages WHERE account_id = ? AND mailbox = 'INBOX'",
        )
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(n, 20);
    }
}
