use anyhow::Result;
use sqlx::{Row, SqlitePool};

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
            smtp_host: r.get::<Option<String>, _>(7).map(|s| crypto::decrypt(&s, key)).transpose()?,
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
            smtp_host: r.get::<Option<String>, _>(7).map(|s| crypto::decrypt(&s, key)).transpose()?,
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
    sqlx::query("DELETE FROM cached_messages WHERE account_id = ?").bind(account_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cached_mailboxes WHERE account_id = ?").bind(account_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cached_attachments WHERE account_id = ?").bind(account_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM virtual_folder_rules WHERE account_id = ?").bind(account_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM imap_accounts WHERE account_id = ? AND user_id = ?")
        .bind(account_id).bind(user_id).execute(&mut *tx).await?;
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

// ── Mailboxes ─────────────────────────────────────────────────────────────────

pub struct MailboxRow {
    pub account_id: String,
    pub name: String,
    pub delimiter: Option<String>,
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
            uidvalidity = excluded.uidvalidity,
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
    let row = sqlx::query("SELECT uidvalidity FROM cached_mailboxes WHERE account_id = ? AND name = ?")
        .bind(account_id)
        .bind(mailbox)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.get::<Option<i64>, _>(0)).map(|v| v as u32))
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
}

pub async fn upsert_message_header(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    h: &MessageHeader,
    subject_idx: Option<&str>,
    from_idx: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cached_messages
            (account_id, mailbox, uid, message_id, thread_id, from_addr, to_addrs, cc_addrs,
             subject, sent_at, flags, has_attachments, subject_idx, from_idx)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            subject_idx     = excluded.subject_idx,
            from_idx        = excluded.from_idx",
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
    .bind(subject_idx)
    .bind(from_idx)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_message_flags(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    flags: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE cached_messages SET flags = ? WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(flags)
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_message_body(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    body_text: &str,
    body_html: Option<&str>,
    body_idx: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE cached_messages SET body_text = ?, body_html = ?, body_idx = ? WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(body_text)
    .bind(body_html)
    .bind(body_idx)
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .execute(pool)
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
    pub body_html: Option<String>,
}

pub async fn list_messages(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<MessageRow>, i64)> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cached_messages WHERE account_id = ? AND mailbox = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query(
        "SELECT uid, account_id, mailbox, message_id, thread_id, from_addr, to_addrs, cc_addrs,
                subject, sent_at, flags, has_attachments, body_text
         FROM cached_messages WHERE account_id = ? AND mailbox = ?
         ORDER BY COALESCE(sent_at, 0) DESC
         LIMIT ? OFFSET ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok((rows.iter().map(row_to_message).collect(), total))
}

pub async fn list_messages_virtual(
    pool: &SqlitePool,
    user_id: &str,
    folder_id: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<MessageRow>, i64)> {
    // Virtual folder joins accounts → rules → messages
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cached_messages m
         JOIN imap_accounts a ON a.account_id = m.account_id
         JOIN virtual_folder_rules r ON r.folder_id = ?
             AND (r.account_id IS NULL OR r.account_id = m.account_id)
             AND (r.mailbox_pattern IS NULL OR m.mailbox LIKE r.mailbox_pattern)
         WHERE a.user_id = ?",
    )
    .bind(folder_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query(
        "SELECT DISTINCT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments, m.body_text
         FROM cached_messages m
         JOIN imap_accounts a ON a.account_id = m.account_id
         JOIN virtual_folder_rules r ON r.folder_id = ?
             AND (r.account_id IS NULL OR r.account_id = m.account_id)
             AND (r.mailbox_pattern IS NULL OR m.mailbox LIKE r.mailbox_pattern)
         WHERE a.user_id = ?
         ORDER BY COALESCE(m.sent_at, 0) DESC
         LIMIT ? OFFSET ?",
    )
    .bind(folder_id)
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok((rows.iter().map(row_to_message).collect(), total))
}

pub async fn get_message(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Result<Option<MessageRow>> {
    let row = sqlx::query(
        "SELECT uid, account_id, mailbox, message_id, thread_id, from_addr, to_addrs, cc_addrs,
                subject, sent_at, flags, has_attachments, body_text, body_html
         FROM cached_messages WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_message_full))
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
        body_html: None,
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
    }
}

pub async fn search_messages(
    pool: &SqlitePool,
    user_id: &str,
    fts_query: &str,
    limit: i64,
) -> Result<Vec<MessageRow>> {
    let rows = sqlx::query(
        "SELECT m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
                m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments, m.body_text
         FROM messages_fts f
         JOIN cached_messages m ON m.rowid = f.rowid
         JOIN imap_accounts a ON a.account_id = m.account_id
         WHERE messages_fts MATCH ?
           AND a.user_id = ?
         ORDER BY rank
         LIMIT ?",
    )
    .bind(fts_query)
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_message).collect())
}

// ── Virtual Folders ───────────────────────────────────────────────────────────

pub struct VirtualFolderRow {
    pub folder_id: String,
    pub display_name: String,
    pub sort_order: i64,
    pub rules: Vec<VirtualFolderRule>,
}

pub struct VirtualFolderRule {
    pub account_id: Option<String>,
    pub mailbox_pattern: Option<String>,
}

pub async fn list_virtual_folders(pool: &SqlitePool, user_id: &str) -> Result<Vec<VirtualFolderRow>> {
    let folders = sqlx::query(
        "SELECT folder_id, display_name, sort_order FROM virtual_folders WHERE user_id = ? ORDER BY sort_order, display_name",
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
            rules: rules
                .iter()
                .map(|r| VirtualFolderRule {
                    account_id: r.get(0),
                    mailbox_pattern: r.get(1),
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
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO virtual_folders (folder_id, user_id, display_name) VALUES (?, ?, ?)",
    )
    .bind(folder_id)
    .bind(user_id)
    .bind(display_name)
    .execute(&mut *tx)
    .await?;

    for (account_id, pattern) in rules {
        sqlx::query(
            "INSERT OR IGNORE INTO virtual_folder_rules (folder_id, account_id, mailbox_pattern) VALUES (?, ?, ?)",
        )
        .bind(folder_id)
        .bind(account_id)
        .bind(pattern)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn delete_virtual_folder(pool: &SqlitePool, folder_id: &str, user_id: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM virtual_folder_rules WHERE folder_id = ?").bind(folder_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM virtual_folders WHERE folder_id = ? AND user_id = ?")
        .bind(folder_id).bind(user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

// ── Image origin whitelist ────────────────────────────────────────────────────

pub async fn get_image_whitelist(pool: &SqlitePool, user_id: &str) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT origin FROM email_image_whitelist WHERE user_id = ? ORDER BY origin",
    )
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
    sqlx::query(
        "INSERT OR IGNORE INTO email_image_whitelist (user_id, origin) VALUES (?, ?)",
    )
    .bind(user_id)
    .bind(origin)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_image_origin(pool: &SqlitePool, user_id: &str, origin: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM email_image_whitelist WHERE user_id = ? AND origin = ?",
    )
    .bind(user_id)
    .bind(origin)
    .execute(pool)
    .await?;
    Ok(())
}
