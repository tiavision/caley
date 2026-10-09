use anyhow::Result;
use sqlx::{Row, SqlitePool};

use crate::crypto;

pub(crate) mod folders_db;
pub(crate) mod messages_db;

pub use folders_db::{
    add_image_blacklist, add_image_origin, create_virtual_folder, delete_virtual_folder,
    get_image_blacklist, get_image_whitelist, is_origin_whitelisted, list_virtual_folders,
    remove_image_blacklist, remove_image_origin, update_virtual_folder,
};
pub use messages_db::{
    AttachmentRow, MessageBodyRow, MessageHeader, MessageRow, PageCursor, PageDirection,
    SearchFilters, delete_attachments_by_uids, delete_attachments_for_mailbox,
    delete_attachments_for_message, delete_message, delete_messages_by_uids, find_trash_mailbox,
    get_attachment_by_cid, get_attachment_by_filename, get_cached_message_flags, get_cached_uids,
    get_cached_uids_missing_body, get_cached_uids_sent_before, get_message,
    get_message_render_body, insert_attachment, list_attachments, list_messages,
    set_message_reply_to,
    list_messages_virtual, list_thread_messages, replace_message_addresses, search_messages,
    search_messages_mailbox, search_messages_virtual, suggest_addrs, update_message_body,
    update_message_flags, upsert_message_header, upsert_message_render_body,
};

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
            "UPDATE email_identities SET is_default = 0 WHERE user_id = ? AND identity_id != ?",
        )
        .bind(user_id)
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

// ── Unread counts ─────────────────────────────────────────────────────────────

/// Returns (account_id, mailbox, unread_count) for all mailboxes with at least
/// one unread message.
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
/// unread message.
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
