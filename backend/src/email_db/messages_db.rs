use anyhow::Result;
use std::collections::HashMap;

use sqlx::{AssertSqlSafe, QueryBuilder, Row, Sqlite, SqlitePool};

// ── Message types ─────────────────────────────────────────────────────────────

pub struct MessageHeader {
    pub uid: i64,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    pub from_addr: Option<String>,
    pub to_addrs: Option<String>,
    pub cc_addrs: Option<String>,
    /// JSON array of Reply-To addresses; `[]` when the header is absent.
    pub reply_to_addrs: Option<String>,
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
    pub rowid: i64,
    #[allow(dead_code)]
    pub size: Option<i64>,
    pub is_encrypted: bool,
    /// JSON array of Reply-To addresses; NULL for rows cached before it was tracked.
    pub reply_to_addrs: Option<String>,
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

pub struct AttachmentRow {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    pub part_index: i64,
    pub content_id: Option<String>,
}

pub struct MessageBodyRow {
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub autocrypt_headers: Vec<String>,
    pub is_signed: bool,
}

pub struct SearchFilters {
    pub addrs: Vec<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub unread_only: bool,
}

impl SearchFilters {
    pub fn is_empty(&self) -> bool {
        self.addrs.is_empty()
            && self.date_from.is_none()
            && self.date_to.is_none()
            && !self.unread_only
    }
}

// ── Message operations ────────────────────────────────────────────────────────

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
             reply_to_addrs, subject, sent_at, flags, has_attachments, size, subject_idx,
             from_idx, to_idx, is_unread)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id, mailbox, uid) DO UPDATE SET
            message_id      = excluded.message_id,
            thread_id       = excluded.thread_id,
            from_addr       = excluded.from_addr,
            to_addrs        = excluded.to_addrs,
            cc_addrs        = excluded.cc_addrs,
            reply_to_addrs  = excluded.reply_to_addrs,
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
    .bind(&h.reply_to_addrs)
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

/// Replaces the normalized address rows for one message.
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

pub async fn get_cached_message_flags(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
) -> Result<HashMap<u32, Option<String>>> {
    let rows =
        sqlx::query("SELECT uid, flags FROM cached_messages WHERE account_id = ? AND mailbox = ?")
            .bind(account_id)
            .bind(mailbox)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<i64, _>(0) as u32, r.get(1)))
        .collect())
}

pub async fn update_message_body<'e, E>(
    executor: E,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    body_idx: &str,
    has_attachments: bool,
    is_encrypted: bool,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "UPDATE cached_messages SET body_idx = ?, has_attachments = ?, is_encrypted = ? WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(body_idx)
    .bind(has_attachments as i64)
    .bind(is_encrypted as i64)
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn upsert_message_render_body<'e, E>(
    executor: E,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    body_text: Option<&str>,
    body_html: Option<&str>,
    autocrypt_headers: &[String],
    is_signed: bool,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO cached_message_bodies
            (account_id, mailbox, uid, body_text, body_html, autocrypt_headers, is_signed)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id, mailbox, uid) DO UPDATE SET
            body_text = excluded.body_text,
            body_html = excluded.body_html,
            autocrypt_headers = excluded.autocrypt_headers,
            is_signed = excluded.is_signed",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .bind(body_text)
    .bind(body_html)
    .bind(serde_json::to_string(autocrypt_headers)?)
    .bind(is_signed as i64)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_message_render_body(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Result<Option<MessageBodyRow>> {
    let row = sqlx::query(
        "SELECT body_text, body_html, autocrypt_headers, is_signed
         FROM cached_message_bodies WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let autocrypt_json: String = r.get(2);
        MessageBodyRow {
            body_text: r.get(0),
            body_html: r.get(1),
            autocrypt_headers: serde_json::from_str(&autocrypt_json).unwrap_or_default(),
            is_signed: r.get::<i64, _>(3) != 0,
        }
    }))
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
    if uids.is_empty() {
        return Ok(());
    }
    let placeholders = uids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "DELETE FROM cached_messages WHERE account_id = ? AND mailbox = ? AND uid IN ({placeholders})"
    );
    let mut q = sqlx::query(AssertSqlSafe(sql))
        .bind(account_id)
        .bind(mailbox);
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
    let mut q = sqlx::query(AssertSqlSafe(sql))
        .bind(account_id)
        .bind(mailbox);
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

pub async fn get_cached_uids_sent_before(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    cutoff_ts: i64,
) -> Result<Vec<u32>> {
    let rows = sqlx::query(
        "SELECT uid FROM cached_messages
         WHERE account_id = ? AND mailbox = ? AND sent_at IS NOT NULL AND sent_at < ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(cutoff_ts)
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
        "SELECT uid FROM cached_messages WHERE account_id = ? AND mailbox = ? AND body_idx IS NULL",
    )
    .bind(account_id)
    .bind(mailbox)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>(0) as u32).collect())
}

// ── Attachments ───────────────────────────────────────────────────────────────

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
        "SELECT filename, content_type, size, part_index, content_id
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
            filename: r.get(0),
            content_type: r.get(1),
            size: r.get(2),
            part_index: r.get(3),
            content_id: r.get(4),
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
        "SELECT filename, content_type, size, part_index, content_id
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
        filename: r.get(0),
        content_type: r.get(1),
        size: r.get(2),
        part_index: r.get(3),
        content_id: r.get(4),
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
        "SELECT filename, content_type, size, part_index, content_id
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
        filename: r.get(0),
        content_type: r.get(1),
        size: r.get(2),
        part_index: r.get(3),
        content_id: r.get(4),
    }))
}

/// Returns the name of the first mailbox that looks like a Trash folder.
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

// ── Keyset pagination helpers ─────────────────────────────────────────────────

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

// ── Message listing ───────────────────────────────────────────────────────────

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

    if let Some(off) = offset {
        let rows = sqlx::query(
            "SELECT uid, account_id, mailbox, message_id, thread_id, from_addr, to_addrs, cc_addrs,
                    subject, sent_at, flags, has_attachments, rowid, is_encrypted, reply_to_addrs
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
                m.rowid, m.is_encrypted, m.reply_to_addrs
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
                    m.rowid, m.is_encrypted, m.reply_to_addrs
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
                m.rowid, m.is_encrypted, m.reply_to_addrs
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
                subject, sent_at, flags, has_attachments, size, rowid, is_encrypted, reply_to_addrs
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
                m.rowid, m.is_encrypted, m.reply_to_addrs
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
        rowid: r.get(12),
        size: None,
        is_encrypted: r.get::<i64, _>(13) != 0,
        reply_to_addrs: r.get(14),
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
        size: r.get(12),
        rowid: r.get(13),
        is_encrypted: r.get::<i64, _>(14) != 0,
        reply_to_addrs: r.get(15),
    }
}

/// Stores Reply-To addresses parsed lazily for rows cached before the column existed.
pub async fn set_message_reply_to(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    reply_to_addrs: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE cached_messages SET reply_to_addrs = ? WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(reply_to_addrs)
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Search ────────────────────────────────────────────────────────────────────

fn push_filter_clauses(qb: &mut QueryBuilder<Sqlite>, filters: &SearchFilters) {
    if filters.unread_only {
        qb.push(" AND m.is_unread = 1");
    }
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

/// Where a search looks: every account of the user, one mailbox, or a virtual folder's rules.
pub enum SearchScope<'a> {
    All,
    Mailbox { account_id: &'a str, mailbox: &'a str },
    Virtual { folder_id: &'a str },
}

/// The search's `FROM … WHERE …`: the user's messages in `scope` matching `fts_query` (all of them when empty)
/// and `filters`. The same for the page and for the count.
fn push_search_body(
    qb: &mut QueryBuilder<Sqlite>,
    user_id: &str,
    scope: &SearchScope<'_>,
    fts_query: &str,
    filters: &SearchFilters,
) {
    if fts_query.is_empty() {
        qb.push(" FROM cached_messages m");
    } else {
        qb.push(" FROM messages_fts f JOIN cached_messages m ON m.rowid = f.rowid");
    }
    qb.push(" JOIN imap_accounts a ON a.account_id = m.account_id");
    if let SearchScope::Virtual { folder_id } = scope {
        qb.push(" JOIN virtual_folder_rules r ON r.folder_id = ")
            .push_bind(folder_id.to_string())
            .push(
                " AND (r.account_id = '' OR r.account_id = m.account_id)
                   AND (r.mailbox_pattern = '' OR m.mailbox LIKE r.mailbox_pattern)
                  JOIN virtual_folders vf ON vf.folder_id = r.folder_id",
            );
    }
    qb.push(" WHERE a.user_id = ").push_bind(user_id.to_string());
    if !fts_query.is_empty() {
        qb.push(" AND messages_fts MATCH ").push_bind(fts_query.to_string());
    }
    match scope {
        SearchScope::All => {}
        SearchScope::Mailbox { account_id, mailbox } => {
            qb.push(" AND m.account_id = ")
                .push_bind(account_id.to_string())
                .push(" AND m.mailbox = ")
                .push_bind(mailbox.to_string());
        }
        SearchScope::Virtual { .. } => {
            qb.push(
                " AND (vf.only_flagged = 0 OR EXISTS (SELECT 1 FROM json_each(m.flags) WHERE value = '\\Flagged'))
                   AND (vf.only_attachments = 0 OR m.has_attachments = 1)",
            );
        }
    }
    push_filter_clauses(qb, filters);
}

/// One page of a search (keyset, as the mailbox list) and the number of messages the search finds in all.
pub async fn search_messages(
    pool: &SqlitePool,
    user_id: &str,
    scope: SearchScope<'_>,
    fts_query: &str,
    filters: &SearchFilters,
    limit: i64,
    cursor: Option<PageCursor>,
    direction: PageDirection,
) -> Result<(Vec<MessageRow>, i64)> {
    // a message can match several rules of a virtual folder: counted and listed once
    let distinct = matches!(scope, SearchScope::Virtual { .. });
    let mut count = QueryBuilder::<Sqlite>::new(if distinct {
        "SELECT COUNT(DISTINCT m.rowid)"
    } else {
        "SELECT COUNT(*)"
    });
    push_search_body(&mut count, user_id, &scope, fts_query, filters);
    let total: i64 = count.build_query_scalar().fetch_one(pool).await?;

    let mut qb = QueryBuilder::<Sqlite>::new(if distinct { "SELECT DISTINCT" } else { "SELECT" });
    qb.push(
        " m.uid, m.account_id, m.mailbox, m.message_id, m.thread_id, m.from_addr,
          m.to_addrs, m.cc_addrs, m.subject, m.sent_at, m.flags, m.has_attachments,
          m.rowid, m.is_encrypted, m.reply_to_addrs",
    );
    push_search_body(&mut qb, user_id, &scope, fts_query, filters);
    match direction {
        PageDirection::Forward => {
            if let Some(ref c) = cursor {
                push_keyset_forward(&mut qb, c);
            }
            qb.push(" ORDER BY m.sent_at DESC, m.rowid DESC LIMIT ")
                .push_bind(limit);
        }
        PageDirection::Backward | PageDirection::Last => {
            if let Some(ref c) = cursor {
                push_keyset_backward(&mut qb, c);
            }
            qb.push(" ORDER BY m.sent_at ASC, m.rowid ASC LIMIT ")
                .push_bind(limit);
        }
    }
    let rows = qb.build().fetch_all(pool).await?;
    let mut messages: Vec<MessageRow> = rows.iter().map(row_to_message).collect();
    if matches!(direction, PageDirection::Backward | PageDirection::Last) {
        messages.reverse();
    }
    Ok((messages, total))
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
                rowid_override INTEGER, is_unread INTEGER NOT NULL DEFAULT 1,
                is_encrypted INTEGER NOT NULL DEFAULT 0, reply_to_addrs TEXT,
                PRIMARY KEY (account_id, mailbox, uid)
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE cached_message_bodies (
                account_id TEXT NOT NULL, mailbox TEXT NOT NULL, uid INTEGER NOT NULL,
                body_text TEXT, body_html TEXT, autocrypt_headers TEXT NOT NULL DEFAULT '[]',
                is_signed INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account_id, mailbox, uid),
                FOREIGN KEY (account_id, mailbox, uid)
                    REFERENCES cached_messages(account_id, mailbox, uid) ON DELETE CASCADE
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn stores_and_reads_render_body() {
        let pool = thread_pool().await;
        insert_thread_message(&pool, "account", "INBOX", 1, "message", None, 1).await;
        let headers = vec!["addr=alice@example.test; keydata=YWJj".to_owned()];

        upsert_message_render_body(
            &pool,
            "account",
            "INBOX",
            1,
            Some("Hello"),
            Some("<p>Hello</p>"),
            &headers,
            true,
        )
        .await
        .unwrap();

        let body = get_message_render_body(&pool, "account", "INBOX", 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(body.body_text.as_deref(), Some("Hello"));
        assert_eq!(body.body_html.as_deref(), Some("<p>Hello</p>"));
        assert_eq!(body.autocrypt_headers, headers);
        assert!(body.is_signed);
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

    #[tokio::test]
    async fn update_message_flags_keeps_unread_index_in_sync() {
        let pool = thread_pool().await;
        insert_thread_message(&pool, "account", "INBOX", 1, "msg", None, 1).await;

        update_message_flags(&pool, "account", "INBOX", 1, r#"["\\Seen"]"#)
            .await
            .unwrap();
        let is_unread: i64 =
            sqlx::query_scalar("SELECT is_unread FROM cached_messages WHERE uid = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(is_unread, 0);

        update_message_flags(&pool, "account", "INBOX", 1, r#"[]"#)
            .await
            .unwrap();
        let is_unread: i64 =
            sqlx::query_scalar("SELECT is_unread FROM cached_messages WHERE uid = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(is_unread, 1);
    }
}
