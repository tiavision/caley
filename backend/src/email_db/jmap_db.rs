//! What the cache keeps of a JMAP account beyond the IMAP tables (migration 0029): its mailboxes' ids, the email
//! behind each cached message, and the states the sync is at.

use anyhow::Result;
use sqlx::{Row, SqlitePool};

pub struct JmapMailboxRow {
    pub mailbox_id: String,
    pub name: String,
}

pub async fn jmap_mailboxes(pool: &SqlitePool, account_id: &str) -> Result<Vec<JmapMailboxRow>> {
    let rows = sqlx::query(
        "SELECT mailbox_id, name FROM jmap_mailboxes WHERE account_id = ? ORDER BY name",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| JmapMailboxRow {
            mailbox_id: r.get(0),
            name: r.get(1),
        })
        .collect())
}

/// Adds a mailbox, or renames it (its next UID is kept).
pub async fn upsert_jmap_mailbox(
    pool: &SqlitePool,
    account_id: &str,
    mailbox_id: &str,
    name: &str,
    role: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO jmap_mailboxes (account_id, mailbox_id, name, role) VALUES (?, ?, ?, ?)
         ON CONFLICT(account_id, mailbox_id) DO UPDATE SET name = excluded.name, role = excluded.role",
    )
    .bind(account_id)
    .bind(mailbox_id)
    .bind(name)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_jmap_mailbox(
    pool: &SqlitePool,
    account_id: &str,
    mailbox_id: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM jmap_mailboxes WHERE account_id = ? AND mailbox_id = ?")
        .bind(account_id)
        .bind(mailbox_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Drops a mailbox's row from the cached mailboxes (the IMAP table the mailbox list reads).
pub async fn delete_cached_mailbox(pool: &SqlitePool, account_id: &str, name: &str) -> Result<()> {
    sqlx::query("DELETE FROM cached_mailboxes WHERE account_id = ? AND name = ?")
        .bind(account_id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

/// The id of the mailbox the cache calls `name`.
pub async fn jmap_mailbox_id(
    pool: &SqlitePool,
    account_id: &str,
    name: &str,
) -> Result<Option<String>> {
    let row =
        sqlx::query("SELECT mailbox_id FROM jmap_mailboxes WHERE account_id = ? AND name = ?")
            .bind(account_id)
            .bind(name)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.get(0)))
}

/// The mailbox with a role (`sent`, `trash`, `drafts`): its id and name.
pub async fn jmap_mailbox_by_role(
    pool: &SqlitePool,
    account_id: &str,
    role: &str,
) -> Result<Option<(String, String)>> {
    let row = sqlx::query(
        "SELECT mailbox_id, name FROM jmap_mailboxes WHERE account_id = ? AND role = ? ORDER BY name LIMIT 1",
    )
    .bind(account_id)
    .bind(role)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.get(0), r.get(1))))
}

/// Gives the next UID of a mailbox (never one given before).
pub async fn next_uid(pool: &SqlitePool, account_id: &str, name: &str) -> Result<u32> {
    let row = sqlx::query(
        "UPDATE jmap_mailboxes SET next_uid = next_uid + 1 WHERE account_id = ? AND name = ?
         RETURNING next_uid - 1",
    )
    .bind(account_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok(r.get::<i64, _>(0) as u32),
        None => Err(anyhow::anyhow!(
            "no JMAP mailbox {name} in account {account_id}"
        )),
    }
}

/// The cached messages an email is: (mailbox, UID) for each mailbox it is in.
pub async fn jmap_memberships(
    pool: &SqlitePool,
    account_id: &str,
    email_id: &str,
) -> Result<Vec<(String, u32)>> {
    let rows = sqlx::query(
        "SELECT mailbox, uid FROM jmap_messages WHERE account_id = ? AND email_id = ? ORDER BY mailbox",
    )
    .bind(account_id)
    .bind(email_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get(0), r.get::<i64, _>(1) as u32))
        .collect())
}

/// Records the email behind a cached message (its row in cached_messages must exist).
pub async fn insert_jmap_message(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
    email_id: &str,
    blob_id: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO jmap_messages (account_id, mailbox, uid, email_id, blob_id) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(account_id, mailbox, uid) DO UPDATE SET email_id = excluded.email_id, blob_id = excluded.blob_id",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .bind(email_id)
    .bind(blob_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// The email behind a cached message: (email id, blob id).
pub async fn jmap_message(
    pool: &SqlitePool,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Result<Option<(String, String)>> {
    let row = sqlx::query(
        "SELECT email_id, blob_id FROM jmap_messages WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.get(0), r.get(1))))
}

/// Every email id the cache has of an account.
pub async fn jmap_email_ids(pool: &SqlitePool, account_id: &str) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT DISTINCT email_id FROM jmap_messages WHERE account_id = ?")
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// The messages of an account whose body has not been synced yet: (mailbox, UID, blob id).
pub async fn jmap_messages_missing_body(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<Vec<(String, u32, String)>> {
    let rows = sqlx::query(
        "SELECT j.mailbox, j.uid, j.blob_id FROM jmap_messages j
         JOIN cached_messages m ON m.account_id = j.account_id AND m.mailbox = j.mailbox AND m.uid = j.uid
         WHERE j.account_id = ? AND m.body_idx IS NULL
         ORDER BY j.blob_id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get(0), r.get::<i64, _>(1) as u32, r.get(2)))
        .collect())
}

pub async fn jmap_state(pool: &SqlitePool, account_id: &str, kind: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT state FROM jmap_states WHERE account_id = ? AND type = ?")
        .bind(account_id)
        .bind(kind)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get(0)))
}

pub async fn set_jmap_state(
    pool: &SqlitePool,
    account_id: &str,
    kind: &str,
    state: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO jmap_states (account_id, type, state) VALUES (?, ?, ?)
         ON CONFLICT(account_id, type) DO UPDATE SET state = excluded.state",
    )
    .bind(account_id)
    .bind(kind)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}
