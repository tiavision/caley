//! Demo mode seed data.
//!
//! Called once on startup when DEMO_MODE=true. Credentials and account rows use
//! INSERT … ON CONFLICT DO UPDATE so stale data from a previous run (e.g. before
//! an encryption-key change) is always corrected. Content rows (events, messages,
//! contacts) still use INSERT OR IGNORE so user edits survive a restart.

use sqlx::SqlitePool;

pub(crate) mod calendars;
pub(crate) mod contacts;
pub(crate) mod email;

pub const DEMO_USER_ID: &str = "demo@demo.example";
pub(super) const DEMO_CALDAV_ACCOUNT: &str = "demo-caldav";
pub(super) const DEMO_CALDAV_URL: &str = "https://demo.caldav.example";
pub(super) const DEMO_WORK_ACCOUNT: &str = "demo-work";
pub(super) const DEMO_PERSONAL_ACCOUNT: &str = "demo-personal";
pub(super) const DEMO_ADDRESSBOOK_HREF: &str = "/addressbooks/demo/contacts/";
pub(super) const DEMO_WORK_CAL: &str = "/calendars/demo/work/";
pub(super) const DEMO_PERSONAL_CAL: &str = "/calendars/demo/personal/";
pub(super) const DEMO_FAMILY_CAL: &str = "/calendars/demo/family/";

pub async fn seed(pool: &SqlitePool, encryption_key: &[u8; 32]) -> anyhow::Result<()> {
    println!("[demo] seeding demo database for user {DEMO_USER_ID}");
    seed_user(pool).await?;
    seed_caldav_account(pool, encryption_key).await?;
    calendars::seed_calendars(pool).await?;
    calendars::seed_events(pool).await?;
    contacts::seed_addressbook(pool).await?;
    contacts::seed_contacts(pool).await?;
    email::seed_imap_accounts(pool, encryption_key).await?;
    email::seed_mailboxes(pool).await?;
    email::seed_messages(pool).await?;
    Ok(())
}

async fn seed_user(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO user_settings (user_id, display_name) VALUES (?, ?)",
    )
    .bind(DEMO_USER_ID)
    .bind("Demo User")
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_caldav_account(pool: &SqlitePool, key: &[u8; 32]) -> anyhow::Result<()> {
    let enc_url = crate::crypto::encrypt(DEMO_CALDAV_URL, key)?;
    let enc_user = crate::crypto::encrypt("demo", key)?;
    let enc_pass = crate::crypto::encrypt("demo", key)?;
    sqlx::query(
        "INSERT INTO caldav_accounts
            (account_id, user_id, display_name, caldav_base_url, caldav_username, caldav_password, sort_order)
         VALUES (?, ?, ?, ?, ?, ?, 0)
         ON CONFLICT(account_id) DO UPDATE SET
            caldav_base_url = excluded.caldav_base_url,
            caldav_username = excluded.caldav_username,
            caldav_password = excluded.caldav_password",
    )
    .bind(DEMO_CALDAV_ACCOUNT)
    .bind(DEMO_USER_ID)
    .bind("Demo CalDAV")
    .bind(enc_url)
    .bind(enc_user)
    .bind(enc_pass)
    .execute(pool)
    .await?;
    Ok(())
}
