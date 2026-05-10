use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

pub struct UserSettings {
    pub caldav_base_url: String,
    pub caldav_username: String,
    pub caldav_password: String,
}

pub type Db = Mutex<Connection>;

fn encrypt_field(value: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    crate::crypto::encrypt(value, key)
}

fn decrypt_field(value: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    crate::crypto::decrypt(value, key)
}

pub fn init_db(path: &str) -> anyhow::Result<Connection> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS user_settings (
            user_id TEXT PRIMARY KEY,
            caldav_base_url TEXT NOT NULL,
            caldav_username TEXT NOT NULL,
            caldav_password TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS deactivated_calendars (
            user_id TEXT NOT NULL,
            calendar_href TEXT NOT NULL,
            PRIMARY KEY (user_id, calendar_href)
        );",
    )?;
    Ok(conn)
}

pub fn get_deactivated_calendars(db: &Connection, user_id: &str, key: &[u8; 32]) -> anyhow::Result<HashSet<String>> {
    let mut stmt = db.prepare(
        "SELECT calendar_href FROM deactivated_calendars WHERE user_id = ?1",
    )?;
    let rows = stmt.query_map(params![user_id], |row| row.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for row in rows {
        let enc_href = row?;
        set.insert(decrypt_field(&enc_href, key)?);
    }
    Ok(set)
}

pub fn deactivate_calendar(db: &Connection, user_id: &str, calendar_href: &str, key: &[u8; 32]) -> anyhow::Result<()> {
    let enc_href = encrypt_field(calendar_href, key)?;
    db.execute(
        "INSERT OR IGNORE INTO deactivated_calendars (user_id, calendar_href) VALUES (?1, ?2)",
        params![user_id, enc_href],
    )?;
    Ok(())
}

pub fn activate_calendar(db: &Connection, user_id: &str, calendar_href: &str, key: &[u8; 32]) -> anyhow::Result<()> {
    let enc_href = encrypt_field(calendar_href, key)?;
    db.execute(
        "DELETE FROM deactivated_calendars WHERE user_id = ?1 AND calendar_href = ?2",
        params![user_id, enc_href],
    )?;
    Ok(())
}

pub fn get_settings(db: &Connection, user_id: &str, key: &[u8; 32]) -> anyhow::Result<Option<UserSettings>> {
    let mut stmt = db.prepare(
        "SELECT caldav_base_url, caldav_username, caldav_password FROM user_settings WHERE user_id = ?1",
    )?;
    let result = stmt.query_row(params![user_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    });
    match result {
        Ok((enc_url, enc_user, enc_pass)) => {
            let caldav_base_url = decrypt_field(&enc_url, key)?;
            let caldav_username = decrypt_field(&enc_user, key)?;
            let caldav_password = decrypt_field(&enc_pass, key)?;
            Ok(Some(UserSettings {
                caldav_base_url,
                caldav_username,
                caldav_password,
            }))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn upsert_settings(
    db: &Connection,
    user_id: &str,
    settings: &UserSettings,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let enc_url = encrypt_field(&settings.caldav_base_url, key)?;
    let enc_user = encrypt_field(&settings.caldav_username, key)?;
    let enc_pass = encrypt_field(&settings.caldav_password, key)?;
    db.execute(
        "INSERT INTO user_settings (user_id, caldav_base_url, caldav_username, caldav_password)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(user_id) DO UPDATE SET
            caldav_base_url = excluded.caldav_base_url,
            caldav_username = excluded.caldav_username,
            caldav_password = excluded.caldav_password",
        params![user_id, enc_url, enc_user, enc_pass],
    )?;
    Ok(())
}