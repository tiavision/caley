//! The JMAP client, sync and operations against the fake server (`fake.rs`). `stalwart_tests.rs` runs the same kinds
//! of checks against a real Stalwart.

use std::sync::Arc;

use serde_json::json;
use sqlx::Row;
use tokio::sync::broadcast;

use super::fake::{ACCOUNT, FakeJmap};
use super::*;
use crate::email_db::{self, ImapAccountRow, jmap_db};
use crate::email_sync::{EmailSyncManager, eml_path};
use crate::oidc_tokens::OidcTokens;
use crate::sse::{SseBroadcaster, SseMessage};

// ── Fixtures ─────────────────────────────────────────────────────────────────

pub(crate) fn mail(subject: &str, from: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: user@example.com\r\nSubject: {subject}\r\nMessage-ID: <{}@example.com>\r\n\
         Date: Tue, 24 Nov 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\
         \r\nHello from {subject}.\r\n",
        uuid::Uuid::new_v4()
    )
    .into_bytes()
}

pub(super) fn mail_with_attachment(subject: &str) -> Vec<u8> {
    format!(
        "From: Ann <ann@example.org>\r\nTo: user@example.com\r\nSubject: {subject}\r\n\
         Message-ID: <{}@example.org>\r\nDate: Tue, 24 Nov 2026 11:00:00 +0000\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain\r\n\r\n\
         See the file.\r\n--b1\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\n\
         Content-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n\
         JVBERi0xLjQK\r\n--b1--\r\n",
        uuid::Uuid::new_v4()
    )
    .into_bytes()
}

/// A JMAP account in a fresh database, its sync manager, the fake server and the user's server events.
struct Env {
    fake: FakeJmap,
    mgr: Arc<EmailSyncManager>,
    db: sqlx::SqlitePool,
    dir: std::path::PathBuf,
    events: broadcast::Receiver<SseMessage>,
    tokens: Arc<OidcTokens>,
}

const USER: &str = "u@example.com";
const ACC: &str = "jmap-acc";
const KEY: [u8; 32] = [3; 32];

async fn env() -> Env {
    let fake = FakeJmap::start().await;
    env_with(fake, "password").await
}

async fn env_with(fake: FakeJmap, auth: &str) -> Env {
    let dir = std::env::temp_dir().join(format!("caley-jmap-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = crate::db::init_db(dir.join("t.db").to_str().unwrap())
        .await
        .unwrap();
    crate::db::ensure_user(&db, USER, None).await.unwrap();
    let (user, pass) = match auth {
        "password" => ("user", "secret"),
        "bearer" => ("", "api-token"),
        _ => ("", ""),
    };
    email_db::upsert_account(
        &db,
        USER,
        ACC,
        "Fake",
        "127.0.0.1",
        80,
        user,
        pass,
        false,
        None,
        None,
        &email_db::AccountProtocol {
            protocol: "jmap",
            jmap_url: Some(&fake.url),
            auth, oauth_client_id: None, oauth_client_secret: None,
        },
        &KEY,
    )
    .await
    .unwrap();
    let broadcaster = Arc::new(SseBroadcaster::new());
    let events = broadcaster.subscribe(USER);
    let tokens = Arc::new(OidcTokens::new(db.clone(), KEY, None));
    let mgr = EmailSyncManager::new(
        db.clone(),
        KEY,
        broadcaster,
        60,
        crate::search::SearchMode::Default,
        dir.clone(),
        false,
        Arc::clone(&tokens),
    );
    Env {
        fake,
        mgr,
        db,
        dir,
        events,
        tokens,
    }
}

impl Env {
    async fn account(&self) -> ImapAccountRow {
        email_db::get_account(&self.db, ACC, USER, &KEY)
            .await
            .unwrap()
            .unwrap()
    }

    async fn sync(&self) {
        self.mgr
            .jmap_sync(&self.account().await, USER, None)
            .await
            .unwrap();
    }

    async fn client(&self) -> Client {
        self.mgr
            .jmap_client(&self.account().await, USER)
            .await
            .unwrap()
    }

    fn drain(&mut self) -> Vec<SseMessage> {
        let mut out = Vec::new();
        while let Ok(m) = self.events.try_recv() {
            out.push(m);
        }
        out
    }

    /// (uid, subject, flags) of a cached mailbox, by UID.
    async fn cached(&self, mailbox: &str) -> Vec<(u32, String, Vec<String>)> {
        let rows = sqlx::query(
            "SELECT uid, subject, flags FROM cached_messages WHERE account_id = ? AND mailbox = ? ORDER BY uid",
        )
        .bind(ACC)
        .bind(mailbox)
        .fetch_all(&self.db)
        .await
        .unwrap();
        rows.iter()
            .map(|r| {
                let flags: Option<String> = r.get(2);
                (
                    r.get::<i64, _>(0) as u32,
                    r.get::<Option<String>, _>(1).unwrap_or_default(),
                    flags
                        .and_then(|f| serde_json::from_str(&f).ok())
                        .unwrap_or_default(),
                )
            })
            .collect()
    }

    async fn mailboxes(&self) -> Vec<String> {
        email_db::list_mailboxes(&self.db, crate::email_db::MailAccountId::internal(ACC))
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect()
    }

    fn eml(&self, mailbox: &str, uid: u32) -> Option<Vec<u8>> {
        let bytes = std::fs::read(eml_path(
            &self.dir,
            crate::email_db::MailAccountId::internal(ACC),
            mailbox,
            uid,
        ))
        .ok()?;
        Some(zstd::decode_all(bytes.as_slice()).unwrap())
    }
}

fn subjects(rows: &[(u32, String, Vec<String>)]) -> Vec<&str> {
    rows.iter().map(|(_, s, _)| s.as_str()).collect()
}

// ── Pure parts ───────────────────────────────────────────────────────────────

#[test]
fn session_urls_from_what_was_typed() {
    assert_eq!(
        session_url("mail.example.com").unwrap().as_str(),
        "https://mail.example.com/.well-known/jmap"
    );
    assert_eq!(
        session_url("mail.example.com:8443").unwrap().as_str(),
        "https://mail.example.com:8443/.well-known/jmap"
    );
    assert_eq!(
        session_url("https://mail.example.com/jmap/session")
            .unwrap()
            .as_str(),
        "https://mail.example.com/jmap/session"
    );
    assert_eq!(
        session_url(" http://localhost:8080 ").unwrap().as_str(),
        "http://localhost:8080/.well-known/jmap"
    );
    assert!(session_url("").is_err());
    assert!(session_url("ftp://x").is_err());
}

#[test]
fn keywords_are_imap_flags_and_back() {
    for (kw, flag) in [
        ("$seen", "\\Seen"),
        ("$flagged", "\\Flagged"),
        ("$answered", "\\Answered"),
        ("$draft", "\\Draft"),
        ("$forwarded", "$Forwarded"),
        ("$junk", "$junk"),
    ] {
        assert_eq!(keyword_to_flag(kw), flag);
        assert_eq!(flag_to_keyword(flag).as_deref(), Some(kw));
    }
    assert_eq!(keyword_to_flag("$Seen"), "\\Seen");
    assert_eq!(flag_to_keyword("\\Deleted"), None);
    assert_eq!(flag_to_keyword("\\Recent"), None);
    assert_eq!(flag_to_keyword(""), None);
    assert_eq!(pointer_escape("a/b~c"), "a~1b~0c");
}

#[test]
fn only_set_keywords_are_flags() {
    let email: Email = serde_json::from_value(json!({
        "id": "E1", "blobId": "B1",
        "keywords": { "$seen": true, "$flagged": false, "$answered": true, "$hasnoattachment": true },
    }))
    .unwrap();
    assert_eq!(
        email.flags(),
        vec!["\\Answered", "\\Seen"],
        "set ones, not the server's own"
    );
}

#[test]
fn header_fields_become_the_header_section_the_parser_reads() {
    let email: Email = serde_json::from_value(json!({
        "id": "E1", "blobId": "B1",
        "headers": [
            { "name": "From", "value": " Ann <ann@example.org>" },
            { "name": "Subject", "value": " a long\r\n subject" },
            { "name": "Message-ID", "value": " <m1@example.org>" },
            { "name": "In-Reply-To", "value": " <m0@example.org>" },
        ],
    }))
    .unwrap();
    let h =
        crate::imap_client::header_from_bytes(7, &email.header_bytes(), &email.flags(), Some(10));
    assert_eq!(h.uid, 7);
    assert_eq!(h.subject.as_deref(), Some("a long subject"));
    assert_eq!(h.from_emails, vec!["ann@example.org"]);
    assert_eq!(h.message_id.as_deref(), Some("m1@example.org"));
    assert_eq!(h.thread_id.as_deref(), Some("m0@example.org"));
    assert_eq!(h.flags.as_deref(), Some("[]"));
}

fn mb(id: &str, name: &str, parent: Option<&str>, role: Option<&str>) -> Mailbox {
    Mailbox {
        id: id.into(),
        name: name.into(),
        parent_id: parent.map(Into::into),
        role: role.map(Into::into),
        is_subscribed: true,
    }
}

#[test]
fn mailbox_names_are_paths_with_the_inbox_as_imap_names_it() {
    let names = mailbox_names(&[
        mb("a", "Inbox", None, Some("inbox")),
        mb("b", "Projects", Some("a"), None),
        mb("c", "Archive", None, Some("archive")),
        mb("d", "2026", Some("c"), None),
        mb("e", "x/y", None, None),
        mb("f", "Orphan", Some("missing"), None),
    ]);
    assert_eq!(names["a"], "INBOX");
    assert_eq!(names["b"], "INBOX/Projects");
    assert_eq!(names["c"], "Archive");
    assert_eq!(names["d"], "Archive/2026");
    assert_eq!(names["e"], "x∕y", "a slash in a name is not a level");
    assert_eq!(names["f"], "Orphan");
}

#[test]
fn equal_paths_and_a_folder_called_inbox_get_their_ids() {
    let names = mailbox_names(&[
        mb("a", "Inbox", None, Some("inbox")),
        mb("b", "inbox", None, None),
        mb("c", "Lists", None, None),
        mb("d", "Lists", None, None),
    ]);
    assert_eq!(names["a"], "INBOX");
    assert_eq!(names["b"], "inbox (b)");
    assert_eq!(names["c"], "Lists");
    assert_eq!(names["d"], "Lists (d)");
}

#[test]
fn a_parent_cycle_does_not_loop() {
    let names = mailbox_names(&[mb("a", "A", Some("b"), None), mb("b", "B", Some("a"), None)]);
    assert_eq!(names.len(), 2);
}

#[test]
fn event_streams_are_parsed_across_chunks() {
    let mut p = EventStreamParser::default();
    assert!(p.push(b": ping\n\nevent: sta").is_empty());
    assert!(p.push(b"te\ndata: {\"a\":").is_empty());
    let events = p.push(b"1}\r\n\r\nevent: ping\ndata: {}\n\n");
    assert_eq!(
        events,
        vec![
            ServerEvent {
                event: "state".into(),
                data: "{\"a\":1}".into()
            },
            ServerEvent {
                event: "ping".into(),
                data: "{}".into()
            },
        ]
    );
    let mut p = EventStreamParser::default();
    let events = p.push(b"data: a\ndata: b\n\n");
    assert_eq!(events[0].data, "a\nb");
}

#[test]
fn state_changes_name_the_types_of_this_account() {
    let event = ServerEvent {
        event: "state".into(),
        data: json!({ "@type": "StateChange", "changed": {
            "acc1": { "Email": "s5", "Mailbox": "m2" }, "other": { "Email": "s9" },
        } })
        .to_string(),
    };
    let changed = state_change(&event, "acc1").unwrap();
    assert_eq!(changed["Email"], "s5");
    assert_eq!(changed["Mailbox"], "m2");
    assert!(state_change(&event, "nobody").is_none());
    let ping = ServerEvent {
        event: "ping".into(),
        data: "{\"interval\":300}".into(),
    };
    assert!(state_change(&ping, "acc1").is_none());
}

#[test]
fn identities_by_address_then_wildcard() {
    let ids = vec![
        ("i1".to_owned(), "user@example.com".to_owned()),
        ("i2".to_owned(), "*@example.com".to_owned()),
    ];
    assert_eq!(
        ops::identity_for(&ids, "User@Example.com").as_deref(),
        Some("i1")
    );
    assert_eq!(
        ops::identity_for(&ids, "alias@example.com").as_deref(),
        Some("i2")
    );
    assert_eq!(ops::identity_for(&ids, "x@other.org"), None);
}

// ── The client ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn connects_through_the_well_known_redirect_and_resolves_relative_urls() {
    let fake = FakeJmap::start().await;
    let http = http_client().unwrap();
    let client = Client::connect(&http, &fake.url, fake.auth())
        .await
        .unwrap();
    assert_eq!(client.account_id, ACCOUNT);
    assert_eq!(client.session.username.as_deref(), Some("user@example.com"));
    let (mailboxes, _) = client.mailboxes().await.unwrap();
    assert_eq!(mailboxes.len(), 5);
    assert!(
        client
            .event_source_url("Email", 300)
            .unwrap()
            .unwrap()
            .as_str()
            .starts_with(&fake.url)
    );
}

#[tokio::test]
async fn a_wrong_password_is_unauthorized() {
    let fake = FakeJmap::start().await;
    let http = http_client().unwrap();
    let wrong = Auth::Basic {
        username: "user".into(),
        password: "nope".into(),
    };
    let e = Client::connect(&http, &fake.url, wrong)
        .await
        .err()
        .unwrap();
    assert!(is_unauthorized(&e), "{e}");
}

#[tokio::test]
async fn queries_and_gets_page_by_max_objects() {
    let fake = FakeJmap::start().await;
    fake.set_max_objects(3);
    let inbox = fake.mailbox_id("inbox");
    for i in 0..7 {
        fake.deliver(&[&inbox], &mail(&format!("m{i}"), "a@example.org"), &[]);
    }
    let http = http_client().unwrap();
    let client = Client::connect(&http, &fake.url, fake.auth())
        .await
        .unwrap();
    let ids = client.query_ids(serde_json::Value::Null).await.unwrap();
    assert_eq!(ids.len(), 7);
    let (emails, state) = client.emails(&ids).await.unwrap();
    assert_eq!(emails.len(), 7);
    assert!(!state.is_empty());
}

#[tokio::test]
async fn changes_come_page_by_page() {
    let fake = FakeJmap::start().await;
    fake.set_max_objects(2);
    let inbox = fake.mailbox_id("inbox");
    let http = http_client().unwrap();
    let client = Client::connect(&http, &fake.url, fake.auth())
        .await
        .unwrap();
    let since = client.state("Email").await.unwrap();
    let a = fake.deliver(&[&inbox], &mail("a", "x@example.org"), &[]);
    let b = fake.deliver(&[&inbox], &mail("b", "x@example.org"), &[]);
    let c = fake.deliver(&[&inbox], &mail("c", "x@example.org"), &[]);
    fake.set_keyword(&a, "$seen", true);
    fake.destroy(&b);
    let ch = client.changes("Email", &since).await.unwrap();
    // three pages of two states: a and b created, then c created and a updated, then b destroyed
    assert_eq!(ch.created, vec![a.clone(), b.clone(), c]);
    assert_eq!(ch.updated, vec![a]);
    assert_eq!(ch.destroyed, vec![b]);
    assert_eq!(ch.new_state, client.state("Email").await.unwrap());
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| *c == "Email/changes")
            .count(),
        3
    );
}

#[tokio::test]
async fn a_forgotten_state_is_cannot_calculate_changes() {
    let fake = FakeJmap::start().await;
    let http = http_client().unwrap();
    let client = Client::connect(&http, &fake.url, fake.auth())
        .await
        .unwrap();
    let since = client.state("Email").await.unwrap();
    fake.deliver(
        &[&fake.mailbox_id("inbox")],
        &mail("a", "x@example.org"),
        &[],
    );
    fake.forget_changes();
    let e = client.changes("Email", &since).await.unwrap_err();
    assert_eq!(method_error(&e), Some("cannotCalculateChanges"));
}

#[tokio::test]
async fn push_reports_state_changes() {
    let fake = FakeJmap::start().await;
    let http = http_client().unwrap();
    let client = Client::connect(&http, &fake.url, fake.auth())
        .await
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let listener = tokio::spawn(async move {
        client
            .listen("Email", 300, move |changed| {
                let _ = tx.send(changed.clone());
            })
            .await
    });
    // the listener subscribes once its request has arrived: push until it hears one
    let got = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            fake.push_change();
            if let Ok(Some(changed)) =
                tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await
            {
                return changed;
            }
        }
    })
    .await
    .unwrap();
    assert!(got.contains_key("Email"));
    listener.abort();
}

// ── The sync ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_first_sync_caches_mailboxes_messages_and_sources() {
    let mut env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let sent = env.fake.mailbox_id("sent");
    let raw = mail_with_attachment("Report");
    env.fake.deliver(
        &[&inbox],
        &mail("Hello", "Bob <bob@example.org>"),
        &["$seen"],
    );
    env.fake.deliver(&[&inbox], &raw, &["$flagged"]);
    env.fake
        .deliver(&[&sent], &mail("Sent one", "user@example.com"), &["$seen"]);
    env.sync().await;

    let mut names = env.mailboxes().await;
    names.sort();
    assert_eq!(
        names,
        vec![
            "Deleted Items",
            "Drafts",
            "INBOX",
            "Junk Mail",
            "Sent Items"
        ]
    );

    let inbox_rows = env.cached("INBOX").await;
    let mut inbox_subjects = subjects(&inbox_rows);
    inbox_subjects.sort();
    assert_eq!(inbox_subjects, vec!["Hello", "Report"]);
    let report = inbox_rows.iter().find(|r| r.1 == "Report").unwrap();
    assert_eq!(report.2, vec!["\\Flagged"]);
    let hello = inbox_rows.iter().find(|r| r.1 == "Hello").unwrap();
    assert_eq!(hello.2, vec!["\\Seen"]);
    assert_eq!(subjects(&env.cached("Sent Items").await), vec!["Sent one"]);

    // the source as the server has it, its attachment listed, the body indexed
    assert_eq!(env.eml("INBOX", report.0).unwrap(), raw);
    let atts = email_db::list_attachments(
        &env.db,
        crate::email_db::MailAccountId::internal(ACC),
        "INBOX",
        report.0,
    )
    .await
    .unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0].filename.as_deref(), Some("report.pdf"));
    let row = email_db::get_message(
        &env.db,
        crate::email_db::MailAccountId::internal(ACC),
        "INBOX",
        report.0,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(row.has_attachments);
    assert_eq!(row.from_addr.as_deref(), Some("Ann <ann@example.org>"));

    // nothing announced as new on the first sync
    let events = env.drain();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SseMessage::NewEmail { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SseMessage::MailboxesChanged { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SseMessage::EmailsRefetch { mailbox, .. } if mailbox == "INBOX"))
    );
}

#[tokio::test]
async fn later_syncs_bring_new_mail_flag_changes_and_removals() {
    let mut env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let first = env
        .fake
        .deliver(&[&inbox], &mail("First", "a@example.org"), &[]);
    let gone = env
        .fake
        .deliver(&[&inbox], &mail("Gone", "a@example.org"), &[]);
    env.sync().await;
    let before = env.cached("INBOX").await;
    let gone_uid = before.iter().find(|r| r.1 == "Gone").unwrap().0;
    env.drain();

    env.fake
        .deliver(&[&inbox], &mail("New", "Carol <carol@example.org>"), &[]);
    env.fake.set_keyword(&first, "$seen", true);
    env.fake.destroy(&gone);
    env.sync().await;

    let rows = env.cached("INBOX").await;
    let mut s = subjects(&rows);
    s.sort();
    assert_eq!(s, vec!["First", "New"]);
    assert_eq!(
        rows.iter().find(|r| r.1 == "First").unwrap().2,
        vec!["\\Seen"]
    );
    assert!(
        env.eml("INBOX", gone_uid).is_none(),
        "the removed message's file is gone"
    );

    let events = env.drain();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SseMessage::NewEmail { subject: Some(s), .. } if s == "New"))
    );
    assert!(events.iter().any(|e| matches!(e, SseMessage::EmailUpdated { flags, .. } if flags == &vec!["\\Seen".to_owned()])));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SseMessage::EmailRemoved { uid, .. } if *uid == gone_uid))
    );
    // only the changes were asked for: no listing on the second sync
    let calls = env.fake.calls();
    assert_eq!(calls.iter().filter(|c| *c == "Email/query").count(), 1);
    assert!(calls.contains(&"Email/changes".to_owned()));
}

#[tokio::test]
async fn a_moved_message_leaves_one_mailbox_and_gets_a_new_uid_in_the_other() {
    let mut env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let trash = env.fake.mailbox_id("trash");
    let a = env
        .fake
        .deliver(&[&inbox], &mail("Move me", "a@example.org"), &[]);
    env.fake
        .deliver(&[&trash], &mail("Already there", "a@example.org"), &[]);
    env.sync().await;
    let old_uid = env.cached("INBOX").await[0].0;
    env.drain();

    env.fake.move_email(&a, &inbox, &trash);
    env.sync().await;
    assert!(env.cached("INBOX").await.is_empty());
    let trash_rows = env.cached("Deleted Items").await;
    assert_eq!(trash_rows.len(), 2);
    let moved = trash_rows.iter().find(|r| r.1 == "Move me").unwrap();
    assert!(env.eml("Deleted Items", moved.0).is_some());
    let events = env.drain();
    assert!(events.iter().any(|e| matches!(e, SseMessage::EmailRemoved { uid, mailbox, .. } if *uid == old_uid && mailbox == "INBOX")));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SseMessage::NewEmail { .. })),
        "a move is not new mail"
    );

    // and back: the inbox gives it a UID it never gave before
    env.fake.move_email(&a, &trash, &inbox);
    env.sync().await;
    let back = env.cached("INBOX").await;
    assert_eq!(back.len(), 1);
    assert!(back[0].0 > old_uid, "UIDs are not reused");
}

#[tokio::test]
async fn an_email_in_two_mailboxes_is_two_messages_downloaded_once() {
    let env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let lists = env.fake.add_mailbox("Lists", None);
    env.fake
        .deliver(&[&inbox, &lists], &mail("Both", "a@example.org"), &[]);
    env.sync().await;
    assert_eq!(subjects(&env.cached("INBOX").await), vec!["Both"]);
    assert_eq!(subjects(&env.cached("Lists").await), vec!["Both"]);
    assert_eq!(env.fake.downloads().len(), 1);
    let uid = env.cached("Lists").await[0].0;
    assert!(env.eml("Lists", uid).is_some());
}

#[tokio::test]
async fn nested_and_renamed_mailboxes() {
    let mut env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let projects = env.fake.add_mailbox("Projects", Some(&inbox));
    env.fake
        .deliver(&[&projects], &mail("Plan", "a@example.org"), &[]);
    env.sync().await;
    assert_eq!(subjects(&env.cached("INBOX/Projects").await), vec!["Plan"]);
    let mbs = email_db::list_mailboxes(&env.db, crate::email_db::MailAccountId::internal(ACC))
        .await
        .unwrap();
    let p = mbs.iter().find(|m| m.name == "INBOX/Projects").unwrap();
    assert_eq!(p.delimiter.as_deref(), Some("/"));
    env.drain();

    env.fake.rename_mailbox(&projects, "Work");
    env.sync().await;
    assert!(env.cached("INBOX/Projects").await.is_empty());
    assert_eq!(subjects(&env.cached("INBOX/Work").await), vec!["Plan"]);
    let names = env.mailboxes().await;
    assert!(names.contains(&"INBOX/Work".to_owned()));
    assert!(!names.contains(&"INBOX/Projects".to_owned()));
    let uid = env.cached("INBOX/Work").await[0].0;
    assert!(env.eml("INBOX/Work", uid).is_some());
}

#[tokio::test]
async fn a_destroyed_mailbox_leaves_the_cache() {
    let env = env().await;
    let lists = env.fake.add_mailbox("Lists", None);
    env.fake
        .deliver(&[&lists], &mail("Digest", "a@example.org"), &[]);
    env.sync().await;
    assert_eq!(env.cached("Lists").await.len(), 1);
    env.fake.destroy_mailbox(&lists);
    env.sync().await;
    assert!(env.cached("Lists").await.is_empty());
    assert!(!env.mailboxes().await.contains(&"Lists".to_owned()));
}

#[tokio::test]
async fn a_state_the_server_forgot_lists_everything_again() {
    let mut env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let keep = env
        .fake
        .deliver(&[&inbox], &mail("Keep", "a@example.org"), &[]);
    let drop = env
        .fake
        .deliver(&[&inbox], &mail("Drop", "a@example.org"), &[]);
    env.sync().await;
    let keep_uid = env
        .cached("INBOX")
        .await
        .iter()
        .find(|r| r.1 == "Keep")
        .unwrap()
        .0;

    env.fake.destroy(&drop);
    env.fake.set_keyword(&keep, "$flagged", true);
    env.fake
        .deliver(&[&inbox], &mail("Added", "a@example.org"), &[]);
    env.fake.forget_changes();
    env.drain();
    env.sync().await;

    let rows = env.cached("INBOX").await;
    let mut s = subjects(&rows);
    s.sort();
    assert_eq!(s, vec!["Added", "Keep"]);
    let kept = rows.iter().find(|r| r.1 == "Keep").unwrap();
    assert_eq!(kept.0, keep_uid, "a message the cache had keeps its UID");
    assert_eq!(kept.2, vec!["\\Flagged"]);
    let events = env.drain();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SseMessage::NewEmail { .. })),
        "a listing announces nothing"
    );
}

#[tokio::test]
async fn a_failed_download_is_tried_again_with_the_next_sync() {
    let env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let id = env
        .fake
        .deliver(&[&inbox], &mail("Later", "a@example.org"), &[]);
    let blob = env.fake.email(&id).unwrap().blob_id;
    let raw = env.fake.inner.lock().unwrap().blobs.remove(&blob).unwrap();
    env.sync().await;
    let uid = env.cached("INBOX").await[0].0;
    assert!(env.eml("INBOX", uid).is_none());
    env.fake
        .inner
        .lock()
        .unwrap()
        .blobs
        .insert(blob, raw.clone());
    env.sync().await;
    assert_eq!(env.eml("INBOX", uid).unwrap(), raw);
}

#[tokio::test]
async fn an_api_token_is_a_bearer_token() {
    let fake = FakeJmap::start().await;
    fake.require_bearer("api-token");
    let env = env_with(fake, "bearer").await;
    env.fake.deliver(
        &[&env.fake.mailbox_id("inbox")],
        &mail("T", "a@example.org"),
        &[],
    );
    env.sync().await;
    assert_eq!(env.cached("INBOX").await.len(), 1);
}

#[tokio::test]
async fn an_oidc_account_signs_in_with_the_users_access_token() {
    let fake = FakeJmap::start().await;
    fake.require_bearer("users-oidc-access-token");
    let env = env_with(fake, "oidc").await;
    // no sign-in kept yet
    let e = env
        .mgr
        .jmap_sync(&env.account().await, USER, None)
        .await
        .unwrap_err();
    assert!(
        e.downcast_ref::<crate::oidc_tokens::TokenError>().is_some(),
        "{e}"
    );

    env.tokens
        .store(
            USER,
            "users-oidc-access-token",
            Some("r"),
            Some(std::time::Duration::from_secs(3600)),
        )
        .await
        .unwrap();
    env.fake.deliver(
        &[&env.fake.mailbox_id("inbox")],
        &mail("Via OIDC", "a@example.org"),
        &[],
    );
    env.sync().await;
    assert_eq!(subjects(&env.cached("INBOX").await), vec!["Via OIDC"]);

    // a token the server no longer takes
    env.tokens.store(USER, "revoked", None, None).await.unwrap();
    let e = env
        .mgr
        .jmap_sync(&env.account().await, USER, None)
        .await
        .unwrap_err();
    assert!(is_unauthorized(&e), "{e}");
}

#[tokio::test]
async fn a_page_asked_sync_reports_its_mailbox_start_and_end() {
    let mut env = env().await;
    env.mgr
        .jmap_sync(&env.account().await, USER, Some("INBOX"))
        .await
        .unwrap();
    let events = env.drain();
    assert!(
        matches!(events.first(), Some(SseMessage::SyncStarted { mailbox, .. }) if mailbox == "INBOX")
    );
    assert!(
        matches!(events.last(), Some(SseMessage::SyncCompleted { mailbox, .. }) if mailbox == "INBOX")
    );
}

// ── Operations ───────────────────────────────────────────────────────────────

async fn uid_of(env: &Env, mailbox: &str, subject: &str) -> u32 {
    env.cached(mailbox)
        .await
        .iter()
        .find(|r| r.1 == subject)
        .unwrap()
        .0
}

#[tokio::test]
async fn flags_are_set_and_cleared_as_keywords() {
    let env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let id = env
        .fake
        .deliver(&[&inbox], &mail("F", "a@example.org"), &["$flagged"]);
    env.sync().await;
    let uid = uid_of(&env, "INBOX", "F").await;
    let client = env.client().await;
    let seen = vec!["\\Seen".to_owned()];
    let flagged = vec!["\\Flagged".to_owned()];
    let failed = ops::apply_changes(
        &client,
        &env.db,
        ACC,
        &[
            (
                "INBOX",
                uid,
                ops::Change::Flags {
                    flags: &seen,
                    set: true,
                },
            ),
            (
                "INBOX",
                uid,
                ops::Change::Flags {
                    flags: &flagged,
                    set: false,
                },
            ),
        ],
    )
    .await
    .unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    let e = env.fake.email(&id).unwrap();
    assert!(e.keywords.contains("$seen"));
    assert!(!e.keywords.contains("$flagged"));
    // one Email/set for both changes of the message
    assert_eq!(
        env.fake
            .calls()
            .iter()
            .filter(|c| *c == "Email/set")
            .count(),
        1
    );
}

#[tokio::test]
async fn moves_change_the_mailboxes_and_the_next_sync_follows() {
    let env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let archive = env.fake.add_mailbox("Archive", None);
    let id = env
        .fake
        .deliver(&[&inbox], &mail("M", "a@example.org"), &[]);
    env.sync().await;
    let uid = uid_of(&env, "INBOX", "M").await;
    let client = env.client().await;
    let failed = ops::apply_changes(
        &client,
        &env.db,
        ACC,
        &[("INBOX", uid, ops::Change::Move { dest: "Archive" })],
    )
    .await
    .unwrap();
    assert!(failed.is_empty());
    let e = env.fake.email(&id).unwrap();
    assert_eq!(e.mailbox_ids, [archive].into_iter().collect());
    env.sync().await;
    assert!(env.cached("INBOX").await.is_empty());
    assert_eq!(subjects(&env.cached("Archive").await), vec!["M"]);
}

#[tokio::test]
async fn deleting_destroys_an_email_in_one_mailbox_and_unlinks_one_in_several() {
    let env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    let lists = env.fake.add_mailbox("Lists", None);
    let single = env
        .fake
        .deliver(&[&inbox], &mail("Single", "a@example.org"), &[]);
    let both = env
        .fake
        .deliver(&[&inbox, &lists], &mail("Both", "a@example.org"), &[]);
    env.sync().await;
    let client = env.client().await;
    let s = uid_of(&env, "INBOX", "Single").await;
    let b = uid_of(&env, "Lists", "Both").await;
    let failed = ops::apply_changes(
        &client,
        &env.db,
        ACC,
        &[
            ("INBOX", s, ops::Change::Delete),
            ("Lists", b, ops::Change::Delete),
        ],
    )
    .await
    .unwrap();
    assert!(failed.is_empty());
    assert!(env.fake.email(&single).is_none());
    assert_eq!(
        env.fake.email(&both).unwrap().mailbox_ids,
        [inbox].into_iter().collect()
    );
}

#[tokio::test]
async fn a_change_of_an_unknown_message_is_reported_not_sent() {
    let env = env().await;
    env.sync().await;
    let client = env.client().await;
    let failed = ops::apply_changes(
        &client,
        &env.db,
        ACC,
        &[("INBOX", 999, ops::Change::Delete)],
    )
    .await
    .unwrap();
    assert_eq!(failed.len(), 1);
    assert!(!env.fake.calls().contains(&"Email/set".to_owned()));
}

#[tokio::test]
async fn sending_imports_into_sent_and_submits_to_the_envelope() {
    let env = env().await;
    env.sync().await;
    let client = env.client().await;
    let raw = mail("Out", "user@example.com");
    let recipients = vec![
        "bob@example.org".to_owned(),
        "hidden@example.org".to_owned(),
    ];
    let sent = ops::send_raw(
        &client,
        &env.db,
        ACC,
        "user@example.com",
        &recipients,
        raw.clone(),
    )
    .await
    .unwrap();
    assert_eq!(sent, "Sent Items");
    let inner = env.fake.inner.lock().unwrap();
    assert_eq!(inner.submissions.len(), 1);
    let sub = &inner.submissions[0];
    assert_eq!(sub["identityId"], "i1");
    assert_eq!(sub["envelope"]["mailFrom"]["email"], "user@example.com");
    assert_eq!(
        sub["envelope"]["rcptTo"],
        json!([{ "email": "bob@example.org" }, { "email": "hidden@example.org" }])
    );
    let email = &inner.emails[sub["emailId"].as_str().unwrap()];
    assert_eq!(inner.blobs[&email.blob_id], raw);
    assert!(email.keywords.contains("$seen"));
    let sent_id = inner
        .mailboxes
        .iter()
        .find(|(_, m)| m.role.as_deref() == Some("sent"))
        .unwrap()
        .0;
    assert!(email.mailbox_ids.contains(sent_id));
}

#[tokio::test]
async fn sending_as_an_address_the_server_has_no_identity_for_is_refused() {
    let env = env().await;
    env.sync().await;
    let client = env.client().await;
    let e = ops::send_raw(
        &client,
        &env.db,
        ACC,
        "other@example.net",
        &["b@example.org".into()],
        mail("x", "y"),
    )
    .await
    .unwrap_err();
    assert!(e.downcast_ref::<ops::SendError>().is_some());
    assert!(env.fake.inner.lock().unwrap().submissions.is_empty());
}

#[tokio::test]
async fn a_refused_submission_removes_the_copy() {
    let env = env().await;
    env.sync().await;
    env.fake.inner.lock().unwrap().refuse_submission = Some("forbiddenToSend".into());
    let client = env.client().await;
    let e = ops::send_raw(
        &client,
        &env.db,
        ACC,
        "user@example.com",
        &["b@example.org".into()],
        mail("x", "y"),
    )
    .await
    .unwrap_err();
    assert_eq!(method_error(&e), Some("forbiddenToSend"));
    assert!(
        env.fake.inner.lock().unwrap().emails.is_empty(),
        "no copy left in Sent"
    );
}

#[tokio::test]
async fn the_sync_and_the_trash_by_role() {
    let env = env().await;
    env.sync().await;
    let trash = jmap_db::jmap_mailbox_by_role(&env.db, ACC, "trash")
        .await
        .unwrap();
    assert_eq!(trash.map(|t| t.1).as_deref(), Some("Deleted Items"));
    let sent = jmap_db::jmap_mailbox_by_role(&env.db, ACC, "sent")
        .await
        .unwrap();
    assert_eq!(sent.map(|t| t.1).as_deref(), Some("Sent Items"));
}

#[tokio::test]
async fn deleting_the_account_drops_its_jmap_rows() {
    let env = env().await;
    env.fake.deliver(
        &[&env.fake.mailbox_id("inbox")],
        &mail("x", "a@example.org"),
        &[],
    );
    env.sync().await;
    email_db::delete_account(&env.db, crate::email_db::MailAccountId::internal(ACC))
        .await
        .unwrap();
    for table in ["jmap_messages", "jmap_mailboxes", "jmap_states"] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                .fetch_one(&env.db)
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
    }
}

#[tokio::test]
async fn push_triggers_a_sync() {
    let env = env().await;
    let inbox = env.fake.mailbox_id("inbox");
    env.fake
        .deliver(&[&inbox], &mail("Before", "a@example.org"), &[]);
    // the account's task syncs once, then only when pushed (the periodic sync is an hour away)
    env.mgr.ensure_task_for(ACC);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while env.cached("INBOX").await.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    env.fake
        .deliver(&[&inbox], &mail("Pushed", "a@example.org"), &[]);
    let ok = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            env.fake.push_change();
            if env.cached("INBOX").await.iter().any(|r| r.1 == "Pushed") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(ok.is_ok(), "the pushed change was synced");
    env.mgr.stop_idle_for(ACC);
    env.mgr.stop_task_for(ACC);
}

#[tokio::test]
async fn a_state_of_another_server_lists_everything_again() {
    let env = env().await;
    env.fake.deliver(
        &[&env.fake.mailbox_id("inbox")],
        &mail("Here", "a@example.org"),
        &[],
    );
    jmap_db::set_jmap_state(&env.db, ACC, "Email", "not-a-state-of-this-server")
        .await
        .unwrap();
    env.sync().await;
    assert_eq!(subjects(&env.cached("INBOX").await), vec!["Here"]);
    assert_ne!(
        jmap_db::jmap_state(&env.db, ACC, "Email")
            .await
            .unwrap()
            .as_deref(),
        Some("not-a-state-of-this-server")
    );
}
