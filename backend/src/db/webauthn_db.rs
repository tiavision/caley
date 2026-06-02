use sqlx::Row;

use super::Db;

pub struct WebAuthnCredRow {
    pub cred_id: String,
    pub passkey: String,
    pub device_label: Option<String>,
    pub has_prf: bool,
}

pub async fn store_webauthn_credential(
    pool: &Db,
    cred_id: &str,
    user_id: &str,
    passkey_json: &str,
    device_label: Option<&str>,
    has_prf: bool,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO webauthn_credentials(cred_id, user_id, passkey, device_label, has_prf, created_at, last_seen)
         VALUES(?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(cred_id) DO UPDATE SET passkey = excluded.passkey, last_seen = excluded.last_seen",
    )
    .bind(cred_id)
    .bind(user_id)
    .bind(passkey_json)
    .bind(device_label)
    .bind(has_prf as i64)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_webauthn_credentials(
    pool: &Db,
    user_id: &str,
) -> anyhow::Result<Vec<WebAuthnCredRow>> {
    let rows = sqlx::query(
        "SELECT cred_id, passkey, device_label, has_prf FROM webauthn_credentials WHERE user_id = ? ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| WebAuthnCredRow {
            cred_id: r.get(0),
            passkey: r.get(1),
            device_label: r.get(2),
            has_prf: r.get::<i64, _>(3) != 0,
        })
        .collect())
}

pub async fn get_webauthn_cred_ids(pool: &Db, user_id: &str) -> anyhow::Result<Vec<String>> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT cred_id FROM webauthn_credentials WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_webauthn_credential(
    pool: &Db,
    cred_id: &str,
    passkey_json: &str,
    sign_count: u32,
    last_seen: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE webauthn_credentials SET passkey = ?, sign_count = ?, last_seen = ? WHERE cred_id = ?",
    )
    .bind(passkey_json)
    .bind(sign_count as i64)
    .bind(last_seen)
    .bind(cred_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_webauthn_credential(
    pool: &Db,
    cred_id: &str,
    user_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM webauthn_credentials WHERE cred_id = ? AND user_id = ?")
        .bind(cred_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
