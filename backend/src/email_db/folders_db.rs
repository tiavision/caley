use anyhow::Result;
use sqlx::{Row, SqlitePool};

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

/// Allows an origin's images; it leaves the ignore list (an origin is on one list at most).
pub async fn add_image_origin(pool: &SqlitePool, user_id: &str, origin: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM email_image_blacklist WHERE user_id = ? AND origin = ?")
        .bind(user_id)
        .bind(origin)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO email_image_whitelist (user_id, origin) VALUES (?, ?)")
        .bind(user_id)
        .bind(origin)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
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

/// Ignores an origin's images; it leaves the allow list (an origin is on one list at most).
pub async fn add_image_blacklist(pool: &SqlitePool, user_id: &str, origin: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM email_image_whitelist WHERE user_id = ? AND origin = ?")
        .bind(user_id)
        .bind(origin)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO email_image_blacklist (user_id, origin) VALUES (?, ?)")
        .bind(user_id)
        .bind(origin)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
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
