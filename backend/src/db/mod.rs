use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::path::Path;

pub use sqlx::SqlitePool as Db;

pub(crate) mod calendar_db;
pub(crate) mod contact_db;
pub(crate) mod pgp_db;
pub(crate) mod webauthn_db;

pub use calendar_db::{
    CachedCalendar, CalDavAccount, CalDavAccountId, activate_calendar, apply_incremental_sync,
    deactivate_calendar, delete_cached_event, delete_caldav_account, delete_calendar_cache,
    delete_event_by_href, ensure_fts_schema, get_cached_calendars, get_caldav_account,
    get_calendar_sync_state, get_deactivated_calendars, get_event_href, get_push_subscription,
    get_raw_events, get_user_and_calendar_from_push_token, list_caldav_accounts,
    remove_stale_calendars, replace_calendar_events, upsert_cached_calendar,
    upsert_cached_event_raw, upsert_caldav_account, upsert_push_subscription,
};
pub use contact_db::{
    CachedAddressbook, CachedContact, apply_incremental_contact_sync, delete_contact_from_cache,
    get_addressbook_sync_state, get_cached_addressbooks, get_contact,
    get_contact_push_subscription, get_contacts, get_user_and_addressbook_from_push_token,
    remove_stale_addressbooks, replace_addressbook_contacts, upsert_cached_addressbook,
    upsert_contact, upsert_contact_push_subscription,
};
pub use pgp_db::{
    delete_pgp_public_key, get_pgp_public_key, get_pgp_public_keys, store_harvested_public_key,
    trust_pgp_public_key, upsert_pgp_public_key,
};
pub use webauthn_db::{
    delete_webauthn_credential, get_webauthn_cred_ids, get_webauthn_credentials,
    store_webauthn_credential, update_webauthn_credential,
};

pub struct UserSettings {
    pub caldav_base_url: String,
    pub caldav_username: String,
    pub caldav_password: String,
}

pub async fn init_db(path: &str) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // NORMAL is durable under WAL and avoids an fsync on every commit, which
        // otherwise lets a write-heavy email sync starve concurrent calendar syncs.
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(30))
        .pragma("foreign_keys", "ON");

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

/// Ensure a user-identity row exists. Called on every successful login so that
/// `user_settings` (referenced by the cache/contacts/email tables) is present
/// even before any CalDAV account is configured.
pub async fn ensure_user(
    pool: &SqlitePool,
    user_id: &str,
    display_name: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_settings (user_id, display_name) VALUES (?, ?)
         ON CONFLICT(user_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(display_name)
    .execute(pool)
    .await?;
    Ok(())
}

/// Return the user's first CalDAV account for legacy code paths (the single-account
/// settings form, `/api/user`'s `has_settings`). New calendar/contact code should use account-specific APIs.
pub async fn get_settings(
    pool: &SqlitePool,
    user_id: &str,
    key: &[u8; 32],
) -> anyhow::Result<Option<UserSettings>> {
    Ok(calendar_db::list_caldav_accounts(pool, user_id, key)
        .await?
        .into_iter()
        .next()
        .map(|a| UserSettings {
            caldav_base_url: a.caldav_base_url,
            caldav_username: a.caldav_username,
            caldav_password: a.caldav_password,
        }))
}

pub async fn upsert_settings(
    pool: &SqlitePool,
    user_id: &str,
    settings: &UserSettings,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let account_id = format!("{user_id}:default");
    calendar_db::upsert_caldav_account(pool, user_id, Some(&account_id), None, settings, key)
        .await?;
    Ok(())
}

pub async fn clear_user_cache(pool: &SqlitePool, user_id: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM cached_calendars
         WHERE account_id IN (SELECT account_id FROM caldav_accounts WHERE user_id = ?)",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM cached_events
         WHERE account_id IN (SELECT account_id FROM caldav_accounts WHERE user_id = ?)",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM push_subscriptions
         WHERE account_id IN (SELECT account_id FROM caldav_accounts WHERE user_id = ?)",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_user(pool: &SqlitePool, user_id: &str) -> anyhow::Result<()> {
    // ON DELETE CASCADE propagates to all child tables.
    sqlx::query("DELETE FROM user_settings WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
