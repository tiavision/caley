use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

pub struct UserSettings {
    pub caldav_base_url: String,
    pub caldav_username: String,
    pub caldav_password: String,
}

pub type Db = Mutex<Connection>;

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
        );",
    )?;
    Ok(conn)
}

pub fn get_settings(db: &Connection, user_id: &str) -> anyhow::Result<Option<UserSettings>> {
    let mut stmt = db.prepare(
        "SELECT caldav_base_url, caldav_username, caldav_password FROM user_settings WHERE user_id = ?1",
    )?;
    let result = stmt.query_row(params![user_id], |row| {
        Ok(UserSettings {
            caldav_base_url: row.get(0)?,
            caldav_username: row.get(1)?,
            caldav_password: row.get(2)?,
        })
    });
    match result {
        Ok(settings) => Ok(Some(settings)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn upsert_settings(
    db: &Connection,
    user_id: &str,
    settings: &UserSettings,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO user_settings (user_id, caldav_base_url, caldav_username, caldav_password)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(user_id) DO UPDATE SET
            caldav_base_url = excluded.caldav_base_url,
            caldav_username = excluded.caldav_username,
            caldav_password = excluded.caldav_password",
        params![
            user_id,
            settings.caldav_base_url,
            settings.caldav_username,
            settings.caldav_password,
        ],
    )?;
    Ok(())
}