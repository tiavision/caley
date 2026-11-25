use sqlx::Row;

use super::Db;

pub struct PgpPublicKeyRow {
    pub fingerprint: String,
    pub email: String,
    pub armored: String,
    pub is_own: bool,
    pub source: String,
    pub trust: String,
}

pub async fn get_pgp_public_keys(
    pool: &Db,
    user_id: &str,
    email: Option<&str>,
) -> anyhow::Result<Vec<PgpPublicKeyRow>> {
    let rows = if let Some(email) = email {
        sqlx::query(
            "SELECT fingerprint, email, armored, is_own, source, trust
             FROM pgp_public_keys
             WHERE user_id = ? AND lower(email) = lower(?)
             ORDER BY is_own DESC, email, fingerprint",
        )
        .bind(user_id)
        .bind(email)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT fingerprint, email, armored, is_own, source, trust
             FROM pgp_public_keys WHERE user_id = ? ORDER BY is_own DESC, email, fingerprint",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?
    };

    Ok(rows
        .into_iter()
        .map(|r| PgpPublicKeyRow {
            fingerprint: r.get(0),
            email: r.get(1),
            armored: r.get(2),
            is_own: r.get::<i64, _>(3) != 0,
            source: r.get(4),
            trust: r.get(5),
        })
        .collect())
}

pub async fn upsert_pgp_public_key(
    pool: &Db,
    user_id: &str,
    fingerprint: &str,
    email: &str,
    armored: &str,
    is_own: bool,
    source: &str,
    trust: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO pgp_public_keys(user_id, fingerprint, email, armored, is_own, source, trust, created_at)
         VALUES(?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, fingerprint) DO UPDATE SET
            armored = excluded.armored,
            email = excluded.email,
            is_own = MAX(pgp_public_keys.is_own, excluded.is_own),
            source = excluded.source,
            trust = excluded.trust",
    )
    .bind(user_id)
    .bind(fingerprint)
    .bind(email)
    .bind(armored)
    .bind(is_own as i64)
    .bind(source)
    .bind(trust)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Stores a key found in the background (Autocrypt, an attachment) as `unverified`. It never touches a stored key
/// the user trusts or one of their own (trust, armored text, address all kept): it adds a key not stored yet, or
/// refreshes one still waiting to be accepted.
pub async fn store_harvested_public_key(
    pool: &Db,
    user_id: &str,
    fingerprint: &str,
    email: &str,
    armored: &str,
    source: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO pgp_public_keys(user_id, fingerprint, email, armored, is_own, source, trust, created_at)
         VALUES(?, ?, ?, ?, 0, ?, 'unverified', ?)
         ON CONFLICT(user_id, fingerprint) DO UPDATE SET
            armored = excluded.armored,
            email = excluded.email,
            source = excluded.source
         WHERE pgp_public_keys.trust <> 'trusted' AND pgp_public_keys.is_own = 0",
    )
    .bind(user_id)
    .bind(fingerprint)
    .bind(email)
    .bind(armored)
    .bind(source)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// The user accepted a stored key: it is trusted from now on, for the address it was stored for. `replace` (a key
/// change the user accepted): the other keys of others stored for that address go. False when the user has no such key.
pub async fn trust_pgp_public_key(
    pool: &Db,
    user_id: &str,
    fingerprint: &str,
    replace: bool,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    let email: Option<String> = sqlx::query_scalar(
        "UPDATE pgp_public_keys SET trust = 'trusted' WHERE user_id = ? AND fingerprint = ? RETURNING email",
    )
    .bind(user_id)
    .bind(fingerprint)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(email) = email else {
        return Ok(false);
    };
    if replace && !email.trim().is_empty() {
        sqlx::query(
            "DELETE FROM pgp_public_keys
             WHERE user_id = ? AND lower(email) = lower(?) AND fingerprint <> ? AND is_own = 0",
        )
        .bind(user_id)
        .bind(&email)
        .bind(fingerprint)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// One stored key of the user's, by fingerprint.
pub async fn get_pgp_public_key(
    pool: &Db,
    user_id: &str,
    fingerprint: &str,
) -> anyhow::Result<Option<PgpPublicKeyRow>> {
    let row = sqlx::query(
        "SELECT fingerprint, email, armored, is_own, source, trust
         FROM pgp_public_keys WHERE user_id = ? AND fingerprint = ?",
    )
    .bind(user_id)
    .bind(fingerprint)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| PgpPublicKeyRow {
        fingerprint: r.get(0),
        email: r.get(1),
        armored: r.get(2),
        is_own: r.get::<i64, _>(3) != 0,
        source: r.get(4),
        trust: r.get(5),
    }))
}

pub async fn delete_pgp_public_key(
    pool: &Db,
    user_id: &str,
    fingerprint: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM pgp_public_keys WHERE user_id = ? AND fingerprint = ?")
        .bind(user_id)
        .bind(fingerprint)
        .execute(pool)
        .await?;
    Ok(())
}
