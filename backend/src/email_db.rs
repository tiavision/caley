use anyhow::Result;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

use crate::crypto;

// ── Account ──────────────────────────────────────────────────────────────────

pub struct ImapAccountRow {
    pub account_id: String,
    pub display_name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub password: String,
    pub use_tls: bool,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub sort_order: i64,
}

pub async fn list_accounts(
    pool: &SqlitePool,
    user_id: &str,
    key: &[u8; 32],
) -> Result<Vec<ImapAccountRow>> {
    let rows = sqlx::query(
        "SELECT account_id, display_name, host, port, username, password, use_tls, smtp_host, smtp_port, sort_order
         FROM imap_accounts WHERE user_id = ? ORDER BY sort_order, display_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::new();
    for r in rows {
        out.push(ImapAccountRow {
            account_id: r.get(0),
            display_name: r.get(1),
            host: crypto::decrypt(r.get::<String, _>(2).as_str(), key)?,
            port: r.get(3),
            username: crypto::decrypt(r.get::<String, _>(4).as_str(), key)?,
            password: crypto::decrypt(r.get::<String, _>(5).as_str(), key)?,
            use_tls: r.get::<i64, _>(6) != 0,
            smtp_host: r
                .get::<Option<String>, _>(7)
                .map(|s| crypto::decrypt(&s, key))
                .transpose()?,
            smtp_port: r.get(8),
            sort_order: r.get(9),
        });
    }
    Ok(out)
}

pub async fn get_account(
    pool: &SqlitePool,
    account_id: &str,
    user_id: &str,
    key: &[u8; 32],
) -> Result<Option<ImapAccountRow>> {
    let r = sqlx::query(
        "SELECT account_id, display_name, host, port, username, password, use_tls, smtp_host, smtp_port, sort_order
         FROM imap_accounts WHERE account_id = ? AND user_id = ?",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    match r {
        None => Ok(None),
        Some(r) => Ok(Some(ImapAccountRow {
            account_id: r.get(0),
            display_name: r.get(1),
            host: crypto::decrypt(r.get::<String, _>(2).as_str(), key)?,
            port: r.get(3),
            username: crypto::decrypt(r.get::<String, _>(4).as_str(), key)?,
            password: crypto::decrypt(r.get::<String, _>(5).as_str(), key)?,
            use_tls: r.get::<i64, _>(6) != 0,
            smtp_host: r
                .get::<Option<String>, _>(7)
                .map(|s| crypto::decrypt(&s, key))
                .transpose()?,
            smtp_port: r.get(8),
            sort_order: r.get(9),
        })),
    }
}

pub async fn upsert_account(
    pool: &SqlitePool,
    user_id: &str,
    account_id: &str,
    display_name: &str,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    use_tls: bool,
    smtp_host: Option<&str>,
    smtp_port: Option<u16>,
    key: &[u8; 32],
) -> Result<()> {
    let enc_host = crypto::encrypt(host, key)?;
    let enc_user = crypto::encrypt(username, key)?;
    let enc_pass = crypto::encrypt(password, key)?;
    let enc_smtp = smtp_host.map(|h| crypto::encrypt(h, key)).transpose()?;

    sqlx::query(
        "INSERT INTO imap_accounts
            (account_id, user_id, display_name, host, port, username, password, use_tls, smtp_host, smtp_port)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id) DO UPDATE SET
            display_name = excluded.display_name,
            host         = excluded.host,
            port         = excluded.port,
            username     = excluded.username,
            password     = excluded.password,
            use_tls      = excluded.use_tls,
            smtp_host    = excluded.smtp_host,
            smtp_port    = excluded.smtp_port",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(display_name)
    .bind(enc_host)
    .bind(port as i64)
    .bind(enc_user)
    .bind(enc_pass)
    .bind(use_tls as i64)
    .bind(enc_smtp)
    .bind(smtp_port.map(|p| p as i64))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_account(pool: &SqlitePool, account_id: &str, user_id: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM cached_messages WHERE account_id = ?")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM cached_mailboxes WHERE account_id = ?")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM cached_attachments WHERE account_id = ?")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM virtual_folder_rules WHERE account_id = ?")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM imap_accounts WHERE account_id = ? AND user_id = ?")
        .bind(account_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get_all_account_ids(pool: &SqlitePool, user_id: &str) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT account_id FROM imap_accounts WHERE user_id = ?")
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

// ── Identities ───────────────────────────────────────────────────────────────

pub struct EmailIdentityRow {
    pub identity_id: String,
    pub account_id: String,
    pub name: String,
    pub email: String,
    pub is_default: bool,
    pub sort_order: i64,
}

pub async fn list_identities(pool: &SqlitePool, user_id: &str) -> Result<Vec<EmailIdentityRow>> {
    let rows = sqlx::query(
        "SELECT identity_id, account_id, name, email, is_default, sort_order
         FROM email_identities WHERE user_id = ? ORDER BY sort_order, name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| EmailIdentityRow {
            identity_id: r.get(0),
            account_id: r.get(1),
            name: r.get(2),
            email: r.get(3),
            is_default: r.get::<i64, _>(4) != 0,
            sort_order: r.get(5),
        })
        .collect())
}

pub async fn get_identity(
    pool: &SqlitePool,
    identity_id: &str,
    user_id: &str,
) -> Result<Option<EmailIdentityRow>> {
    let r = sqlx::query(
        "SELECT identity_id, account_id, name, email, is_default, sort_order
         FROM email_identities WHERE identity_id = ? AND user_id = ?",
    )
    .bind(identity_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(r.map(|r| EmailIdentityRow {
        identity_id: r.get(0),
        account_id: r.get(1),
        name: r.get(2),
        email: r.get(3),
        is_default: r.get::<i64, _>(4) != 0,
        sort_order: r.get(5),
    }))
}

pub async fn upsert_identity(
    pool: &SqlitePool,
    identity_id: &str,
    account_id: &str,
    user_id: &str,
    name: &str,
    email: &str,
    is_default: bool,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    if is_default {
        sqlx::query(
            "UPDATE email_identities SET is_default = 0 WHERE account_id = ? AND identity_id != ?",
        )
        .bind(account_id)
        .bind(identity_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO email_identities (identity_id, account_id, user_id, name, email, is_default)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(identity_id) DO UPDATE SET
            name       = excluded.name,
            email      = excluded.email,
            is_default = excluded.is_default",
    )
    .bind(identity_id)
    .bind(account_id)
    .bind(user_id)
    .bind(name)
    .bind(email)
    .bind(is_default as i64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_identity(pool: &SqlitePool, identity_id: &str, user_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM email_identities WHERE identity_id = ? AND user_id = ?")
        .bind(identity_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Mailboxes ─────────────────────────────────────────────────────────────────

pub struct MailboxRow {
    pub account_id: String,
    pub name: String,
    pub delimiter: Option<String>,
    #[allow(dead_code)]
    pub uidvalidity: Option<i64>,
    pub subscribed: bool,
}

pub async fn list_mailboxes(pool: &SqlitePool, account_id: &str) -> Result<Vec<MailboxRow>> {
    let rows = sqlx::query(
        "SELECT account_id, name, delimiter, uidvalidity, subscribed
         FROM cached_mailboxes WHERE account_id = ? ORDER BY name",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| MailboxRow {
            account_id: r.get(0),
            name: r.get(1),
            delimiter: r.get(2),
            uidvalidity: r.get(3),
            subscribed: r.get::<i64, _>(4) != 0,
        })
        .collect())
}

pub async fn upsert_mailbox(
    pool: &SqlitePool,
    account_id: &str,
    name: &str,
    delimiter: Option<&str>,
    uidvalidity: Option<u32>,
    subscribed: bool,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cached_mailboxes (account_id, name, delimiter, uidvalidity, subscribed)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(account_id, name) DO UPDATE SET
            delimiter   = excluded.delimiter,
            uidvalidity = COALESCE(excluded.uidvalidity, cached_mailboxes.uidvalidity),
            subscribed  = excluded.subscribed,
            last_synced = strftime('%s','now')",
    )
    .bind(account_id)
    .bind(name)
    .bind(delimiter)
    .bind(uidvalidity.map(|v| v as i64))
    .bind(subscribed as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_mailbox_uidvalidity(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
) -> Result<Option<u32>> {
    let row =
        sqlx::query("SELECT uidvalidity FROM cached_mailboxes WHERE account_id = ? AND name = ?")
            .bind(account_id)
            .bind(mailbox)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .and_then(|r| r.get::<Option<i64>, _>(0))
        .map(|v| v as u32))
}

pub async fn delete_messages_for_mailbox(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM cached_messages WHERE account_id = ? AND mailbox = ?")
        .bind(account_id)
        .bind(mailbox)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Messages ──────────────────────────────────────────────────────────────────

pub struct MessageHeader {
    pub uid: i64,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    pub from_addr: Option<String>,
    pub to_addrs: Option<String>,
    pub cc_addrs: Option<String>,
    pub subject: Option<String>,
    pub sent_at: Option<i64>,
    pub flags: Option<String>,
    pub has_attachments: bool,
    pub size: Option<i64>,
    /// Lowercased, display-name-stripped email addresses, by header field.
    pub from_emails: Vec<String>,
    pub to_emails: Vec<String>,
    pub cc_emails: Vec<String>,
}

pub async fn upsert_message_header(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    h: &MessageHeader,
    subject_idx: Option<&str>,
    from_idx: Option<&str>,
    to_idx: Option<&str>,
) -> Result<()> {
    let is_unread = h
        .flags
        .as_deref()
        .map(|f| if f.contains("\\Seen") { 0i64 } else { 1i64 })
        .unwrap_or(1);
    sqlx::query(
        "INSERT INTO cached_messages
            (account_id, mailbox, uid, message_id, thread_id, from_addr, to_addrs, cc_addrs,
             subject, sent_at, flags, has_attachments, size, subject_idx, from_idx, to_idx, is_unread)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id, mailbox, uid) DO UPDATE SET
            message_id      = excluded.message_id,
            thread_id       = excluded.thread_id,
            from_addr       = excluded.from_addr,
            to_addrs        = excluded.to_addrs,
            cc_addrs        = excluded.cc_addrs,
            subject         = excluded.subject,
            sent_at         = excluded.sent_at,
            flags           = excluded.flags,
            has_attachments = excluded.has_attachments,
            size            = excluded.size,
            subject_idx     = excluded.subject_idx,
            from_idx        = excluded.from_idx,
            to_idx          = excluded.to_idx,
            is_unread       = excluded.is_unread",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(h.uid)
    .bind(&h.message_id)
    .bind(&h.thread_id)
    .bind(&h.from_addr)
    .bind(&h.to_addrs)
    .bind(&h.cc_addrs)
    .bind(&h.subject)
    .bind(h.sent_at)
    .bind(&h.flags)
    .bind(h.has_attachments as i64)
    .bind(h.size)
    .bind(subject_idx)
    .bind(from_idx)
    .bind(to_idx)
    .bind(is_unread)
    .execute(pool)
    .await?;
    Ok(())
}

/// Replaces the normalized address rows for one message. Delete-then-insert so a
/// re-fetched UID (header upsert uses ON CONFLICT, which doesn't fire the cascade)
/// doesn't accumulate duplicate rows.
pub async fn replace_message_addresses(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: i64,
    from_emails: &[String],
    to_emails: &[String],
    cc_emails: &[String],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM message_addresses WHERE account_id = ? AND mailbox = ? AND uid = ?")
        .bind(account_id)
        .bind(mailbox)
        .bind(uid)
        .execute(&mut *tx)
        .await?;

    let rows: Vec<(&str, &String)> = from_emails
        .iter()
        .map(|a| ("from", a))
        .chain(to_emails.iter().map(|a| ("to", a)))
        .chain(cc_emails.iter().map(|a| ("cc", a)))
        .collect();
    if !rows.is_empty() {
        let mut qb = QueryBuilder::<Sqlite>::new(
            "INSERT INTO message_addresses (account_id, mailbox, uid, field, addr) ",
        );
        qb.push_values(rows, |mut b, (field, addr)| {
            b.push_bind(account_id)
                .push_bind(mailbox)
                .push_bind(uid)
                .push_bind(field)
                .push_bind(addr);
        });
        qb.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn update_message_flags(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    flags: &str,
) -> Result<()> {
    let is_unread = if flags.contains("\\Seen") { 0i64 } else { 1i64 };
    sqlx::query(
        "UPDATE cached_messages SET flags = ?, is_unread = ? WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(flags)
    .bind(is_unread)
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_message_body<'e, E>(
    executor: E,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    body_text: &str,
    body_html: Option<&str>,
    body_idx: &str,
    has_attachments: bool,
    is_encrypted: bool,
    is_signed: bool,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "UPDATE cached_messages SET body_text = ?, body_html = ?, body_idx = ?, has_attachments = ?, is_encrypted = ?, is_signed = ? WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(body_text)
    .bind(body_html)
    .bind(body_idx)
    .bind(has_attachments as i64)
    .bind(is_encrypted as i64)
    .bind(is_signed as i64)
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn delete_message(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Result<()> {
    sqlx::query("DELETE FROM cached_messages WHERE account_id = ? AND mailbox = ? AND uid = ?")
        .bind(account_id)
        .bind(mailbox)
        .bind(uid as i64)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_messages_by_uids(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uids: &[u32],
) -> Result<()> {
    // Build parameterised IN clause
    if uids.is_empty() {
        return Ok(());
    }
    let placeholders = uids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "DELETE FROM cached_messages WHERE account_id = ? AND mailbox = ? AND uid IN ({placeholders})"
    );
    let mut q = sqlx::query(&sql).bind(account_id).bind(mailbox);
    for uid in uids {
        q = q.bind(*uid as i64);
    }
    q.execute(pool).await?;
    Ok(())
}

pub async fn delete_attachments_by_uids(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uids: &[u32],
) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let placeholders = uids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "DELETE FROM cached_attachments WHERE account_id = ? AND mailbox = ? AND uid IN ({placeholders})"
    );
    let mut q = sqlx::query(&sql).bind(account_id).bind(mailbox);
    for uid in uids {
        q = q.bind(*uid as i64);
    }
    q.execute(pool).await?;
    Ok(())
}

pub async fn delete_attachments_for_mailbox(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM cached_attachments WHERE account_id = ? AND mailbox = ?")
        .bind(account_id)
        .bind(mailbox)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_cached_uids(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
) -> Result<Vec<u32>> {
    let rows = sqlx::query("SELECT uid FROM cached_messages WHERE account_id = ? AND mailbox = ?")
        .bind(account_id)
        .bind(mailbox)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>(0) as u32).collect())
}

pub async fn get_cached_uids_missing_body(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
) -> Result<Vec<u32>> {
    let rows = sqlx::query(
        "SELECT uid FROM cached_messages WHERE account_id = ? AND mailbox = ? AND body_text IS NULL",
    )
    .bind(account_id)
    .bind(mailbox)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>(0) as u32).collect())
}

pub struct MessageRow {
    pub uid: i64,
    pub account_id: String,
    pub mailbox: String,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    pub from_addr: Option<String>,
    pub to_addrs: Option<String>,
    pub cc_addrs: Option<String>,
    pub subject: Option<String>,
    pub sent_at: Option<i64>,
    pub flags: Option<String>,
    pub has_attachments: bool,
    pub body_text: Option<String>,
    pub rowid: i64,
    pub body_html: Option<String>,
    #[allow(dead_code)]
    pub size: Option<i64>,
    pub is_encrypted: bool,
    pub is_signed: bool,
}

pub struct PageCursor {
    pub sent_at: Option<i64>,
    pub rowid: i64,
}

pub enum PageDirection {
    Forward,
    Backward,
    Last,
}

fn push_keyset_forward(qb: &mut QueryBuilder<Sqlite>, cursor: &PageCursor) {
    match cursor.sent_at {
        Some(cs) => {
            qb.push(" AND (m.sent_at < ")
                .push_bind(cs)
                .push(" OR (m.sent_at = ")
                .push_bind(cs)
                .push(" AND m.rowid < ")
                .push_bind(cursor.rowid)
                .push(") OR m.sent_at IS NULL)");
        }
        None => {
            qb.push(" AND m.sent_at IS NULL AND m.rowid < ")
                .push_bind(cursor.rowid);
        }
    }
}

fn push_keyset_backward(qb: &mut QueryBuilder<Sqlite>, cursor: &PageCursor) {
    match cursor.sent_at {
        Some(cs) => {
            qb.push(" AND (m.sent_at > ")
                .push_bind(cs)
                .push(" OR (m.sent_at = ")
                .push_bind(cs)
                .push(" AND m.rowid > ")
                .push_bind(cursor.rowid)
                .push("))");
        }
        None => {
            qb.push(" AND (m.sent_at IS NULL AND m.rowid > ")
                .push_bind(cursor.rowid)
                .push(" OR m.sent_at IS NOT NULL)");
        }
    }
}

// ── Attachments ───────────────────────────────────────────────────────────────

pub struct AttachmentRow {
    pub id: i64,
    pub account_id: String,
    pub mailbox: String,
    pub uid: i64,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    pub part_index: i64,
    pub content_id: Option<String>,
}

pub async fn insert_attachment<'e, E>(
    executor: E,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    filename: &str,
    content_type: &str,
    size: i64,
    part_index: i64,
    content_id: Option<&str>,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO cached_attachments (account_id, mailbox, uid, filename, content_type, size, part_index, content_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .bind(filename)
    .bind(content_type)
    .bind(size)
    .bind(part_index)
    .bind(content_id)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_attachments(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Result<Vec<AttachmentRow>> {
    let rows = sqlx::query(
        "SELECT id, account_id, mailbox, uid, filename, content_type, size, part_index, content_id
         FROM cached_attachments WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| AttachmentRow {
            id: r.get(0),
            account_id: r.get(1),
            mailbox: r.get(2),
            uid: r.get(3),
            filename: r.get(4),
            content_type: r.get(5),
            size: r.get(6),
            part_index: r.get(7),
            content_id: r.get(8),
        })
        .collect())
}

pub async fn get_attachment_by_cid(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    cid: &str,
) -> Result<Option<AttachmentRow>> {
    let row = sqlx::query(
        "SELECT id, account_id, mailbox, uid, filename, content_type, size, part_index, content_id
         FROM cached_attachments
         WHERE account_id = ? AND mailbox = ? AND uid = ? AND content_id = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .bind(cid)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| AttachmentRow {
        id: r.get(0),
        account_id: r.get(1),
        mailbox: r.get(2),
        uid: r.get(3),
        filename: r.get(4),
        content_type: r.get(5),
        size: r.get(6),
        part_index: r.get(7),
        content_id: r.get(8),
    }))
}

pub async fn get_attachment_by_filename(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    filename: &str,
) -> Result<Option<AttachmentRow>> {
    let row = sqlx::query(
        "SELECT id, account_id, mailbox, uid, filename, content_type, size, part_index, content_id
         FROM cached_attachments
         WHERE account_id = ? AND mailbox = ? AND uid = ? AND filename = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .bind(filename)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| AttachmentRow {
        id: r.get(0),
        account_id: r.get(1),
        mailbox: r.get(2),
        uid: r.get(3),
        filename: r.get(4),
        content_type: r.get(5),
        size: r.get(6),
        part_index: r.get(7),
        content_id: r.get(8),
    }))
}

/// Returns the name of the first mailbox that looks like a Trash folder
/// (leaf name contains "trash", "deleted", or "bin"), or `None` if not found.
pub async fn find_trash_mailbox(pool: &SqlitePool, account_id: &str) -> Result<Option<String>> {
    let rows = sqlx::query("SELECT name FROM cached_mailboxes WHERE account_id = ? ORDER BY name")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    for row in &rows {
        let name: String = row.get(0);
        let leaf = name
            .rsplit(|c| c == '/' || c == '.')
            .next()
            .unwrap_or(&name)
            .to_ascii_lowercase();
        if leaf.contains("trash") || leaf.contains("deleted") || leaf.contains("bin") {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

pub async fn delete_attachments_for_message(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Result<()> {
    sqlx::query("DELETE FROM cached_attachments WHERE account_id = ? AND mailbox = ? AND uid = ?")
        .bind(account_id)
        .bind(mailbox)
        .bind(uid as i64)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_messages(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    limit: i64,
    cursor: Option<PageCursor>,
    direction: PageDirection,
    offset: Option<i64>,
) -> Result<(Vec<MessageRow>, i64)> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cached_messages WHERE account_id = ? AND mailbox = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .fetch_one(pool)
    .await?;

    // Offset-based fallback for jump-to-page
    if let Some(off) = offset {
        let rows = sqlx::query(
            "SELECT uid, account_id, mailbox, message_id, thread_id, from_addr, to_addrs, cc_addrs,
                    subject, sent_at, flags, has_attachments, body_text, rowid, is_encrypted, is_signed
             FROM cached_messages
             WHERE account_id = ? AND mailbox = ?
             ORDER BY sent_at DESC, rowid DESC
             LIMIT ? OFFSET ?",
        )
        .bind(account_id)
        .bind(mailbox)
        .bind(limit)
        .bind(off)
        .fetch_all(pool)
        .await?;
        return Ok((rows.iter().map(row_to_message).collect(), total));
    }

    let fetch_limit = match direction {
        PageDirection::Last => ((total - 1).max(0) % limit) + 1,
        _ => limit,
    };

    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments,
                m.body_text, m.rowid, m.is_encrypted, m.is_signed
         FROM cached_messages m
         WHERE m.account_id = ",
    );
    qb.push_bind(account_id)
        .push(" AND m.mailbox = ")
        .push_bind(mailbox);

    match direction {
        PageDirection::Forward => {
            if let Some(ref c) = cursor {
                push_keyset_forward(&mut qb, c);
            }
            qb.push(" ORDER BY m.sent_at DESC, m.rowid DESC LIMIT ")
                .push_bind(fetch_limit);
        }
        PageDirection::Backward => {
            if let Some(ref c) = cursor {
                push_keyset_backward(&mut qb, c);
            }
            qb.push(" ORDER BY m.sent_at ASC, m.rowid ASC LIMIT ")
                .push_bind(fetch_limit);
        }
        PageDirection::Last => {
            qb.push(" ORDER BY m.sent_at ASC, m.rowid ASC LIMIT ")
                .push_bind(fetch_limit);
        }
    }

    let rows = qb.build().fetch_all(pool).await?;
    let mut messages: Vec<MessageRow> = rows.iter().map(row_to_message).collect();
    if matches!(direction, PageDirection::Backward | PageDirection::Last) {
        messages.reverse();
    }
    Ok((messages, total))
}

pub async fn list_messages_virtual(
    pool: &SqlitePool,
    user_id: &str,
    folder_id: &str,
    limit: i64,
    cursor: Option<PageCursor>,
    direction: PageDirection,
    offset: Option<i64>,
) -> Result<(Vec<MessageRow>, i64)> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cached_messages m
         JOIN imap_accounts a ON a.account_id = m.account_id
         JOIN virtual_folder_rules r ON r.folder_id = ?
             AND (r.account_id = '' OR r.account_id = m.account_id)
             AND (r.mailbox_pattern = '' OR m.mailbox LIKE r.mailbox_pattern)
         JOIN virtual_folders vf ON vf.folder_id = r.folder_id
         WHERE a.user_id = ?
           AND (vf.only_flagged = 0 OR EXISTS (SELECT 1 FROM json_each(m.flags) WHERE value = '\\Flagged'))
           AND (vf.only_attachments = 0 OR m.has_attachments = 1)",
    )
    .bind(folder_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    if let Some(off) = offset {
        let rows = sqlx::query(
            "SELECT DISTINCT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                    m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments,
                    m.body_text, m.rowid, m.is_encrypted, m.is_signed
             FROM cached_messages m
             JOIN imap_accounts a ON a.account_id = m.account_id
             JOIN virtual_folder_rules r ON r.folder_id = ?
                 AND (r.account_id = '' OR r.account_id = m.account_id)
                 AND (r.mailbox_pattern = '' OR m.mailbox LIKE r.mailbox_pattern)
             JOIN virtual_folders vf ON vf.folder_id = r.folder_id
             WHERE a.user_id = ?
               AND (vf.only_flagged = 0 OR EXISTS (SELECT 1 FROM json_each(m.flags) WHERE value = '\\Flagged'))
               AND (vf.only_attachments = 0 OR m.has_attachments = 1)
             ORDER BY m.sent_at DESC, m.rowid DESC
             LIMIT ? OFFSET ?",
        )
        .bind(folder_id)
        .bind(user_id)
        .bind(limit)
        .bind(off)
        .fetch_all(pool)
        .await?;
        return Ok((rows.iter().map(row_to_message).collect(), total));
    }

    let fetch_limit = match direction {
        PageDirection::Last => ((total - 1).max(0) % limit) + 1,
        _ => limit,
    };

    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments,
                m.body_text, m.rowid, m.is_encrypted, m.is_signed
         FROM cached_messages m
         JOIN imap_accounts a ON a.account_id = m.account_id
         JOIN virtual_folder_rules r ON r.folder_id = ",
    );
    qb.push_bind(folder_id).push(
        " AND (r.account_id = '' OR r.account_id = m.account_id)
             AND (r.mailbox_pattern = '' OR m.mailbox LIKE r.mailbox_pattern)
         JOIN virtual_folders vf ON vf.folder_id = r.folder_id
         WHERE a.user_id = ",
    );
    qb.push_bind(user_id).push(
        " AND (vf.only_flagged = 0 OR EXISTS (SELECT 1 FROM json_each(m.flags) WHERE value = '\\Flagged'))
           AND (vf.only_attachments = 0 OR m.has_attachments = 1)",
    );

    match direction {
        PageDirection::Forward => {
            if let Some(ref c) = cursor {
                push_keyset_forward(&mut qb, c);
            }
            qb.push(" ORDER BY m.sent_at DESC, m.rowid DESC LIMIT ")
                .push_bind(fetch_limit);
        }
        PageDirection::Backward => {
            if let Some(ref c) = cursor {
                push_keyset_backward(&mut qb, c);
            }
            qb.push(" ORDER BY m.sent_at ASC, m.rowid ASC LIMIT ")
                .push_bind(fetch_limit);
        }
        PageDirection::Last => {
            qb.push(" ORDER BY m.sent_at ASC, m.rowid ASC LIMIT ")
                .push_bind(fetch_limit);
        }
    }

    let rows = qb.build().fetch_all(pool).await?;
    let mut messages: Vec<MessageRow> = rows.iter().map(row_to_message).collect();
    if matches!(direction, PageDirection::Backward | PageDirection::Last) {
        messages.reverse();
    }
    Ok((messages, total))
}

pub async fn get_message(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Result<Option<MessageRow>> {
    let row = sqlx::query(
        "SELECT uid, account_id, mailbox, message_id, thread_id, from_addr, to_addrs, cc_addrs,
                subject, sent_at, flags, has_attachments, body_text, body_html, size, rowid, is_encrypted, is_signed
         FROM cached_messages WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_message_full))
}

pub async fn list_thread_messages(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Result<Vec<MessageRow>> {
    let rows = sqlx::query(
        "WITH RECURSIVE connected(id) AS (
             SELECT message_id FROM cached_messages
              WHERE account_id = ? AND mailbox = ? AND uid = ? AND message_id IS NOT NULL
             UNION
             SELECT thread_id FROM cached_messages
              WHERE account_id = ? AND mailbox = ? AND uid = ? AND thread_id IS NOT NULL
             UNION
             SELECT m.message_id
               FROM cached_messages m JOIN connected c
                 ON m.message_id = c.id OR m.thread_id = c.id
              WHERE m.account_id = ? AND m.message_id IS NOT NULL
             UNION
             SELECT m.thread_id
               FROM cached_messages m JOIN connected c
                 ON m.message_id = c.id OR m.thread_id = c.id
              WHERE m.account_id = ? AND m.thread_id IS NOT NULL
         )
         SELECT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments,
                m.body_text, m.rowid, m.is_encrypted, m.is_signed
           FROM cached_messages m
          WHERE m.account_id = ?
            AND ((m.mailbox = ? AND m.uid = ?)
              OR m.message_id IN (SELECT id FROM connected)
              OR m.thread_id IN (SELECT id FROM connected))
          ORDER BY COALESCE(m.sent_at, 9223372036854775807), m.rowid",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .bind(account_id)
    .bind(account_id)
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_message).collect())
}

fn row_to_message(r: &sqlx::sqlite::SqliteRow) -> MessageRow {
    MessageRow {
        uid: r.get(0),
        account_id: r.get(1),
        mailbox: r.get(2),
        message_id: r.get(3),
        thread_id: r.get(4),
        from_addr: r.get(5),
        to_addrs: r.get(6),
        cc_addrs: r.get(7),
        subject: r.get(8),
        sent_at: r.get(9),
        flags: r.get(10),
        has_attachments: r.get::<i64, _>(11) != 0,
        body_text: r.get(12),
        rowid: r.get(13),
        body_html: None,
        size: None,
        is_encrypted: r.get::<i64, _>(14) != 0,
        is_signed: r.get::<i64, _>(15) != 0,
    }
}

fn row_to_message_full(r: &sqlx::sqlite::SqliteRow) -> MessageRow {
    MessageRow {
        uid: r.get(0),
        account_id: r.get(1),
        mailbox: r.get(2),
        message_id: r.get(3),
        thread_id: r.get(4),
        from_addr: r.get(5),
        to_addrs: r.get(6),
        cc_addrs: r.get(7),
        subject: r.get(8),
        sent_at: r.get(9),
        flags: r.get(10),
        has_attachments: r.get::<i64, _>(11) != 0,
        body_text: r.get(12),
        body_html: r.get(13),
        size: r.get(14),
        rowid: r.get(15),
        is_encrypted: r.get::<i64, _>(16) != 0,
        is_signed: r.get::<i64, _>(17) != 0,
    }
}

pub struct SearchFilters {
    pub addrs: Vec<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
}

impl SearchFilters {
    pub fn is_empty(&self) -> bool {
        self.addrs.is_empty() && self.date_from.is_none() && self.date_to.is_none()
    }
}

fn push_filter_clauses(qb: &mut QueryBuilder<Sqlite>, filters: &SearchFilters) {
    if !filters.addrs.is_empty() {
        qb.push(
            " AND EXISTS (SELECT 1 FROM message_addresses ma
                          WHERE ma.account_id = m.account_id
                            AND ma.mailbox = m.mailbox
                            AND ma.uid = m.uid
                            AND ma.addr IN (",
        );
        let mut sep = qb.separated(", ");
        for addr in &filters.addrs {
            sep.push_bind(addr.trim().to_lowercase());
        }
        qb.push("))");
    }
    if let Some(ts) = filters.date_from {
        qb.push(" AND m.sent_at >= ").push_bind(ts);
    }
    if let Some(ts) = filters.date_to {
        qb.push(" AND m.sent_at <= ").push_bind(ts);
    }
}

pub async fn search_messages(
    pool: &SqlitePool,
    user_id: &str,
    fts_query: &str,
    filters: &SearchFilters,
    limit: i64,
    cursor: Option<PageCursor>,
    direction: PageDirection,
) -> Result<Vec<MessageRow>> {
    let use_fts = !fts_query.is_empty();
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments,
                m.body_text, m.rowid, m.is_encrypted, m.is_signed",
    );
    if use_fts {
        qb.push(
            " FROM messages_fts f
              JOIN cached_messages m ON m.rowid = f.rowid
              JOIN imap_accounts a ON a.account_id = m.account_id
              WHERE messages_fts MATCH ",
        )
        .push_bind(fts_query)
        .push(" AND a.user_id = ")
        .push_bind(user_id);
    } else {
        qb.push(
            " FROM cached_messages m
              JOIN imap_accounts a ON a.account_id = m.account_id
              WHERE a.user_id = ",
        )
        .push_bind(user_id);
    }
    push_filter_clauses(&mut qb, filters);
    match direction {
        PageDirection::Forward => {
            if let Some(ref c) = cursor {
                push_keyset_forward(&mut qb, c);
            }
            qb.push(" ORDER BY COALESCE(m.sent_at, 0) DESC, m.rowid DESC LIMIT ")
                .push_bind(limit);
        }
        PageDirection::Backward | PageDirection::Last => {
            if let Some(ref c) = cursor {
                push_keyset_backward(&mut qb, c);
            }
            qb.push(" ORDER BY COALESCE(m.sent_at, 0) ASC, m.rowid ASC LIMIT ")
                .push_bind(limit);
        }
    }
    let rows = qb.build().fetch_all(pool).await?;
    let mut messages: Vec<MessageRow> = rows.iter().map(row_to_message).collect();
    if matches!(direction, PageDirection::Backward | PageDirection::Last) {
        messages.reverse();
    }
    Ok(messages)
}

pub async fn search_messages_mailbox(
    pool: &SqlitePool,
    user_id: &str,
    account_id: &str,
    mailbox: &str,
    fts_query: &str,
    filters: &SearchFilters,
    limit: i64,
    cursor: Option<PageCursor>,
    direction: PageDirection,
) -> Result<Vec<MessageRow>> {
    let use_fts = !fts_query.is_empty();
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments,
                m.body_text, m.rowid, m.is_encrypted, m.is_signed",
    );
    if use_fts {
        qb.push(
            " FROM messages_fts f
              JOIN cached_messages m ON m.rowid = f.rowid
              JOIN imap_accounts a ON a.account_id = m.account_id
              WHERE messages_fts MATCH ",
        )
        .push_bind(fts_query)
        .push(" AND a.user_id = ")
        .push_bind(user_id);
    } else {
        qb.push(
            " FROM cached_messages m
              JOIN imap_accounts a ON a.account_id = m.account_id
              WHERE a.user_id = ",
        )
        .push_bind(user_id);
    }
    qb.push(" AND m.account_id = ")
        .push_bind(account_id)
        .push(" AND m.mailbox = ")
        .push_bind(mailbox);
    push_filter_clauses(&mut qb, filters);
    match direction {
        PageDirection::Forward => {
            if let Some(ref c) = cursor {
                push_keyset_forward(&mut qb, c);
            }
            qb.push(" ORDER BY COALESCE(m.sent_at, 0) DESC, m.rowid DESC LIMIT ")
                .push_bind(limit);
        }
        PageDirection::Backward | PageDirection::Last => {
            if let Some(ref c) = cursor {
                push_keyset_backward(&mut qb, c);
            }
            qb.push(" ORDER BY COALESCE(m.sent_at, 0) ASC, m.rowid ASC LIMIT ")
                .push_bind(limit);
        }
    }
    let rows = qb.build().fetch_all(pool).await?;
    let mut messages: Vec<MessageRow> = rows.iter().map(row_to_message).collect();
    if matches!(direction, PageDirection::Backward | PageDirection::Last) {
        messages.reverse();
    }
    Ok(messages)
}

pub async fn search_messages_virtual(
    pool: &SqlitePool,
    user_id: &str,
    folder_id: &str,
    fts_query: &str,
    filters: &SearchFilters,
    limit: i64,
    cursor: Option<PageCursor>,
    direction: PageDirection,
) -> Result<Vec<MessageRow>> {
    let use_fts = !fts_query.is_empty();
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments,
                m.body_text, m.rowid, m.is_encrypted, m.is_signed",
    );
    if use_fts {
        qb.push(
            " FROM messages_fts f
              JOIN cached_messages m ON m.rowid = f.rowid
              JOIN imap_accounts a ON a.account_id = m.account_id
              JOIN virtual_folder_rules r ON r.folder_id = ",
        )
        .push_bind(folder_id)
        .push(
            " AND (r.account_id = '' OR r.account_id = m.account_id)
               AND (r.mailbox_pattern = '' OR m.mailbox LIKE r.mailbox_pattern)
              JOIN virtual_folders vf ON vf.folder_id = r.folder_id
              WHERE messages_fts MATCH ",
        )
        .push_bind(fts_query)
        .push(" AND a.user_id = ")
        .push_bind(user_id);
    } else {
        qb.push(
            " FROM cached_messages m
              JOIN imap_accounts a ON a.account_id = m.account_id
              JOIN virtual_folder_rules r ON r.folder_id = ",
        )
        .push_bind(folder_id)
        .push(
            " AND (r.account_id = '' OR r.account_id = m.account_id)
               AND (r.mailbox_pattern = '' OR m.mailbox LIKE r.mailbox_pattern)
              JOIN virtual_folders vf ON vf.folder_id = r.folder_id
              WHERE a.user_id = ",
        )
        .push_bind(user_id);
    }
    qb.push(
        " AND (vf.only_flagged = 0 OR EXISTS (SELECT 1 FROM json_each(m.flags) WHERE value = '\\Flagged'))
           AND (vf.only_attachments = 0 OR m.has_attachments = 1)",
    );
    push_filter_clauses(&mut qb, filters);
    match direction {
        PageDirection::Forward => {
            if let Some(ref c) = cursor {
                push_keyset_forward(&mut qb, c);
            }
            qb.push(" ORDER BY COALESCE(m.sent_at, 0) DESC, m.rowid DESC LIMIT ")
                .push_bind(limit);
        }
        PageDirection::Backward | PageDirection::Last => {
            if let Some(ref c) = cursor {
                push_keyset_backward(&mut qb, c);
            }
            qb.push(" ORDER BY COALESCE(m.sent_at, 0) ASC, m.rowid ASC LIMIT ")
                .push_bind(limit);
        }
    }
    let rows = qb.build().fetch_all(pool).await?;
    let mut messages: Vec<MessageRow> = rows.iter().map(row_to_message).collect();
    if matches!(direction, PageDirection::Backward | PageDirection::Last) {
        messages.reverse();
    }
    Ok(messages)
}

pub async fn suggest_addrs(
    pool: &SqlitePool,
    user_id: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<String>> {
    let pattern = format!("%{}%", query.to_lowercase());
    sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT ma.addr
           FROM message_addresses ma
           JOIN imap_accounts a ON a.account_id = ma.account_id
          WHERE a.user_id = ? AND ma.addr LIKE ?
          LIMIT ?",
    )
    .bind(user_id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(anyhow::Error::from)
}

// ── Virtual Folders ───────────────────────────────────────────────────────────

pub struct VirtualFolderRow {
    pub folder_id: String,
    pub display_name: String,
    pub sort_order: i64,
    pub rules: Vec<VirtualFolderRule>,
    pub only_flagged: bool,
    pub only_attachments: bool,
}

pub struct VirtualFolderRule {
    pub account_id: Option<String>,
    pub mailbox_pattern: Option<String>,
}

pub async fn list_virtual_folders(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<VirtualFolderRow>> {
    let folders = sqlx::query(
        "SELECT folder_id, display_name, sort_order, only_flagged, only_attachments FROM virtual_folders WHERE user_id = ? ORDER BY sort_order, display_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::new();
    for f in &folders {
        let folder_id: String = f.get(0);
        let rules = sqlx::query(
            "SELECT account_id, mailbox_pattern FROM virtual_folder_rules WHERE folder_id = ?",
        )
        .bind(&folder_id)
        .fetch_all(pool)
        .await?;
        out.push(VirtualFolderRow {
            folder_id,
            display_name: f.get(1),
            sort_order: f.get(2),
            only_flagged: f.get::<i64, _>(3) != 0,
            only_attachments: f.get::<i64, _>(4) != 0,
            rules: rules
                .iter()
                .map(|r| VirtualFolderRule {
                    account_id: r
                        .get::<Option<String>, _>(0)
                        .and_then(|s| if s.is_empty() { None } else { Some(s) }),
                    mailbox_pattern: r
                        .get::<Option<String>, _>(1)
                        .and_then(|s| if s.is_empty() { None } else { Some(s) }),
                })
                .collect(),
        });
    }
    Ok(out)
}

pub async fn create_virtual_folder(
    pool: &SqlitePool,
    folder_id: &str,
    user_id: &str,
    display_name: &str,
    rules: &[(Option<&str>, Option<&str>)],
    only_flagged: bool,
    only_attachments: bool,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO virtual_folders (folder_id, user_id, display_name, only_flagged, only_attachments) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(folder_id)
    .bind(user_id)
    .bind(display_name)
    .bind(only_flagged as i64)
    .bind(only_attachments as i64)
    .execute(&mut *tx)
    .await?;

    for (account_id, pattern) in rules {
        sqlx::query(
            "INSERT OR IGNORE INTO virtual_folder_rules (folder_id, account_id, mailbox_pattern) VALUES (?, ?, ?)",
        )
        .bind(folder_id)
        .bind(account_id.unwrap_or(""))
        .bind(pattern.unwrap_or(""))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn update_virtual_folder(
    pool: &SqlitePool,
    folder_id: &str,
    user_id: &str,
    display_name: &str,
    rules: &[(Option<&str>, Option<&str>)],
    only_flagged: bool,
    only_attachments: bool,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "UPDATE virtual_folders SET display_name = ?, only_flagged = ?, only_attachments = ? WHERE folder_id = ? AND user_id = ?",
    )
    .bind(display_name)
    .bind(only_flagged as i64)
    .bind(only_attachments as i64)
    .bind(folder_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if rows.rows_affected() == 0 {
        return Err(anyhow::anyhow!("not found"));
    }
    sqlx::query("DELETE FROM virtual_folder_rules WHERE folder_id = ?")
        .bind(folder_id)
        .execute(&mut *tx)
        .await?;
    for (account_id, pattern) in rules {
        sqlx::query(
            "INSERT OR IGNORE INTO virtual_folder_rules (folder_id, account_id, mailbox_pattern) VALUES (?, ?, ?)",
        )
        .bind(folder_id)
        .bind(account_id.unwrap_or(""))
        .bind(pattern.unwrap_or(""))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn delete_virtual_folder(
    pool: &SqlitePool,
    folder_id: &str,
    user_id: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query("DELETE FROM virtual_folders WHERE folder_id = ? AND user_id = ?")
        .bind(folder_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    if rows.rows_affected() == 0 {
        return Ok(());
    }
    sqlx::query("DELETE FROM virtual_folder_rules WHERE folder_id = ?")
        .bind(folder_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

// ── Image origin whitelist ────────────────────────────────────────────────────

pub async fn get_image_whitelist(pool: &SqlitePool, user_id: &str) -> Result<Vec<String>> {
    let rows =
        sqlx::query("SELECT origin FROM email_image_whitelist WHERE user_id = ? ORDER BY origin")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

pub async fn is_origin_whitelisted(pool: &SqlitePool, user_id: &str, origin: &str) -> Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM email_image_whitelist WHERE user_id = ? AND origin = ?",
    )
    .bind(user_id)
    .bind(origin)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

pub async fn add_image_origin(pool: &SqlitePool, user_id: &str, origin: &str) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO email_image_whitelist (user_id, origin) VALUES (?, ?)")
        .bind(user_id)
        .bind(origin)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn remove_image_origin(pool: &SqlitePool, user_id: &str, origin: &str) -> Result<()> {
    sqlx::query("DELETE FROM email_image_whitelist WHERE user_id = ? AND origin = ?")
        .bind(user_id)
        .bind(origin)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Image origin blacklist ─────────────────────────────────────────────────

pub async fn get_image_blacklist(pool: &SqlitePool, user_id: &str) -> Result<Vec<String>> {
    let rows =
        sqlx::query("SELECT origin FROM email_image_blacklist WHERE user_id = ? ORDER BY origin")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

pub async fn add_image_blacklist(pool: &SqlitePool, user_id: &str, origin: &str) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO email_image_blacklist (user_id, origin) VALUES (?, ?)")
        .bind(user_id)
        .bind(origin)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn remove_image_blacklist(pool: &SqlitePool, user_id: &str, origin: &str) -> Result<()> {
    sqlx::query("DELETE FROM email_image_blacklist WHERE user_id = ? AND origin = ?")
        .bind(user_id)
        .bind(origin)
        .execute(pool)
        .await?;
    Ok(())
}

/// Returns (account_id, mailbox, unread_count) for all mailboxes with at least
/// one unread message. Unread = \\Seen flag absent from the JSON flags array.
pub async fn get_unread_counts(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<(String, String, i64)>> {
    let rows = sqlx::query(
        "SELECT m.account_id, m.mailbox, COUNT(*) as cnt
         FROM cached_messages m
         JOIN imap_accounts a ON a.account_id = m.account_id
         WHERE a.user_id = ?
           AND m.is_unread = 1
         GROUP BY m.account_id, m.mailbox",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect())
}

/// Returns (folder_id, unread_count) for all virtual folders with at least one
/// unread message. Deduplicates messages matched by multiple rules via DISTINCT.
pub async fn get_unread_counts_virtual(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<(String, i64)>> {
    let rows = sqlx::query(
        "SELECT sub.folder_id, COUNT(*) FROM (
             SELECT DISTINCT r.folder_id, m.account_id, m.mailbox, m.uid
             FROM cached_messages m
             JOIN imap_accounts a ON a.account_id = m.account_id
             JOIN virtual_folder_rules r
                 ON (r.account_id = '' OR r.account_id = m.account_id)
                 AND (r.mailbox_pattern = '' OR m.mailbox LIKE r.mailbox_pattern)
             JOIN virtual_folders vf ON vf.folder_id = r.folder_id
             WHERE a.user_id = ?
               AND m.is_unread = 1
               AND (vf.only_flagged = 0 OR EXISTS (SELECT 1 FROM json_each(m.flags) WHERE value = '\\Flagged'))
               AND (vf.only_attachments = 0 OR m.has_attachments = 1)
         ) sub GROUP BY sub.folder_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn thread_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE cached_messages (
                account_id TEXT NOT NULL, mailbox TEXT NOT NULL, uid INTEGER NOT NULL,
                message_id TEXT, thread_id TEXT, from_addr TEXT, to_addrs TEXT, cc_addrs TEXT,
                subject TEXT, sent_at INTEGER, flags TEXT, has_attachments INTEGER NOT NULL DEFAULT 0,
                body_text TEXT, rowid_override INTEGER, is_encrypted INTEGER NOT NULL DEFAULT 0,
                is_signed INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (account_id, mailbox, uid)
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn insert_thread_message(
        pool: &SqlitePool,
        account_id: &str,
        mailbox: &str,
        uid: i64,
        message_id: &str,
        parent: Option<&str>,
        sent_at: i64,
    ) {
        sqlx::query(
            "INSERT INTO cached_messages
             (account_id, mailbox, uid, message_id, thread_id, subject, sent_at, flags)
             VALUES (?, ?, ?, ?, ?, ?, ?, '[]')",
        )
        .bind(account_id)
        .bind(mailbox)
        .bind(uid)
        .bind(message_id)
        .bind(parent)
        .bind(message_id)
        .bind(sent_at)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn thread_lookup_finds_connected_messages_across_mailboxes_in_one_account() {
        let pool = thread_pool().await;
        insert_thread_message(&pool, "account", "INBOX", 1, "root", None, 1).await;
        insert_thread_message(&pool, "account", "Sent", 2, "reply", Some("root"), 2).await;
        insert_thread_message(&pool, "account", "Archive", 3, "leaf", Some("reply"), 3).await;
        insert_thread_message(&pool, "other", "INBOX", 4, "foreign", Some("reply"), 4).await;

        let messages = list_thread_messages(&pool, "account", "Archive", 3)
            .await
            .unwrap();
        let ids: Vec<_> = messages
            .iter()
            .filter_map(|m| m.message_id.as_deref())
            .collect();
        assert_eq!(ids, vec!["root", "reply", "leaf"]);
    }
}
