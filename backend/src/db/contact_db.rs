use sqlx::{Row, SqlitePool};
use std::collections::HashSet;

#[derive(sqlx::FromRow)]
pub struct CachedAddressbook {
    pub account_id: String,
    pub href: String,
    pub name: String,
}

pub async fn upsert_cached_addressbook(
    pool: &SqlitePool,
    account_id: &str,
    href: &str,
    name: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO cached_addressbooks (account_id, href, name, last_synced)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(account_id, href) DO UPDATE SET
            name = excluded.name,
            last_synced = excluded.last_synced",
    )
    .bind(account_id)
    .bind(href)
    .bind(name)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_cached_addressbooks(
    pool: &SqlitePool,
    user_id: &str,
) -> anyhow::Result<Vec<CachedAddressbook>> {
    let books = sqlx::query_as::<_, CachedAddressbook>(
        "SELECT a.account_id, a.href, a.name
         FROM cached_addressbooks a
         JOIN caldav_accounts ca ON ca.account_id = a.account_id
         WHERE ca.user_id = ?
         ORDER BY ca.sort_order, a.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(books)
}

pub async fn get_addressbook_sync_state(
    pool: &SqlitePool,
    account_id: &str,
    href: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let row = sqlx::query(
        "SELECT ctag, sync_token FROM cached_addressbooks WHERE account_id = ? AND href = ?",
    )
    .bind(account_id)
    .bind(href)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok((r.get(0), r.get(1))),
        None => Ok((None, None)),
    }
}

pub async fn remove_stale_addressbooks(
    pool: &SqlitePool,
    account_id: &str,
    fresh_hrefs: &HashSet<String>,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    if !fresh_hrefs.is_empty() {
        let placeholders = fresh_hrefs
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!(
            "DELETE FROM contact_push_subscriptions WHERE account_id = ? AND addressbook_href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql).bind(account_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;

        let sql = format!(
            "DELETE FROM cached_contacts WHERE account_id = ? AND addressbook_href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql).bind(account_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;

        let sql = format!(
            "DELETE FROM cached_addressbooks WHERE account_id = ? AND href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql).bind(account_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;
    } else {
        sqlx::query("DELETE FROM contact_push_subscriptions WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cached_contacts WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cached_addressbooks WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ── Contact cache ─────────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
pub struct CachedContact {
    pub account_id: String,
    pub uid: String,
    pub addressbook_href: String,
    pub href: Option<String>,
    #[allow(dead_code)]
    pub etag: String,
    pub raw_vcard: String,
    pub fn_name: String,
    pub emails: String,    // JSON array of TypedEntry
    pub phones: String,    // JSON array of TypedEntry
    pub addresses: String, // JSON array of AddressEntry
    pub org: String,
    pub photo: Option<String>,
}

pub async fn upsert_contact(
    pool: &SqlitePool,
    account_id: &str,
    addressbook_href: &str,
    uid: &str,
    etag: &str,
    href: &str,
    raw_vcard: &str,
    fn_name: &str,
    emails_json: &str,
    phones_json: &str,
    addresses_json: &str,
    org: &str,
    photo: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO cached_contacts
             (account_id, addressbook_href, uid, etag, href, raw_vcard, fn_name, emails, phones, addresses, org, photo)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id, addressbook_href, uid) DO UPDATE SET
             etag = excluded.etag,
             href = excluded.href,
             raw_vcard = excluded.raw_vcard,
             fn_name = excluded.fn_name,
             emails = excluded.emails,
             phones = excluded.phones,
             addresses = excluded.addresses,
             org = excluded.org,
             photo = excluded.photo",
    )
    .bind(account_id)
    .bind(addressbook_href)
    .bind(uid)
    .bind(etag)
    .bind(href)
    .bind(raw_vcard)
    .bind(fn_name)
    .bind(emails_json)
    .bind(phones_json)
    .bind(addresses_json)
    .bind(org)
    .bind(photo)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_contacts(pool: &SqlitePool, user_id: &str) -> anyhow::Result<Vec<CachedContact>> {
    let contacts = sqlx::query_as::<_, CachedContact>(
        "SELECT c.account_id, c.uid, c.addressbook_href, c.href, c.etag, c.raw_vcard, c.fn_name, c.emails, c.phones, c.addresses, c.org, c.photo
         FROM cached_contacts c
         JOIN caldav_accounts ca ON ca.account_id = c.account_id
         WHERE ca.user_id = ?
         ORDER BY fn_name COLLATE NOCASE ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(contacts)
}

pub async fn get_contact(
    pool: &SqlitePool,
    account_id: &str,
    addressbook_href: &str,
    uid: &str,
) -> anyhow::Result<Option<CachedContact>> {
    let contact = sqlx::query_as::<_, CachedContact>(
        "SELECT account_id, uid, addressbook_href, href, etag, raw_vcard, fn_name, emails, phones, addresses, org, photo
         FROM cached_contacts
         WHERE account_id = ? AND addressbook_href = ? AND uid = ?",
    )
    .bind(account_id)
    .bind(addressbook_href)
    .bind(uid)
    .fetch_optional(pool)
    .await?;
    Ok(contact)
}

pub async fn delete_contact_from_cache(
    pool: &SqlitePool,
    account_id: &str,
    addressbook_href: &str,
    uid: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM cached_contacts WHERE account_id = ? AND addressbook_href = ? AND uid = ?",
    )
    .bind(account_id)
    .bind(addressbook_href)
    .bind(uid)
    .execute(pool)
    .await?;
    Ok(())
}

pub struct ContactRef {
    pub account_id: String,
    pub uid: String,
    pub addressbook_href: String,
}

pub struct ContactSyncApplyResult {
    pub changed: Vec<ContactRef>,
    pub deleted: Vec<ContactRef>,
}

pub async fn apply_incremental_contact_sync(
    pool: &SqlitePool,
    account_id: &str,
    addressbook_href: &str,
    changed: Vec<(String, String, String)>, // (href, etag, raw_vcard)
    deleted: Vec<String>,
    new_token: &str,
    stored_ctag: Option<&str>,
) -> anyhow::Result<ContactSyncApplyResult> {
    use crate::carddav::parse_vcard;
    let mut tx = pool.begin().await?;

    let mut changed_result: Vec<ContactRef> = Vec::new();
    for (href, etag, raw_vcard) in changed {
        let fields = parse_vcard(&raw_vcard);
        if fields.uid.is_empty() {
            continue;
        }
        let emails_json =
            serde_json::to_string(&fields.emails).unwrap_or_else(|_| "[]".to_string());
        let phones_json =
            serde_json::to_string(&fields.phones).unwrap_or_else(|_| "[]".to_string());
        let addresses_json =
            serde_json::to_string(&fields.addresses).unwrap_or_else(|_| "[]".to_string());
        sqlx::query(
            "INSERT INTO cached_contacts
                 (account_id, addressbook_href, uid, etag, href, raw_vcard, fn_name, emails, phones, addresses, org, photo)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, addressbook_href, uid) DO UPDATE SET
                 etag = excluded.etag,
                 href = excluded.href,
                 raw_vcard = excluded.raw_vcard,
                 fn_name = excluded.fn_name,
                 emails = excluded.emails,
                 phones = excluded.phones,
                 addresses = excluded.addresses,
                 org = excluded.org,
                 photo = excluded.photo",
        )
        .bind(account_id)
        .bind(addressbook_href)
        .bind(&fields.uid)
        .bind(&etag)
        .bind(&href)
        .bind(&raw_vcard)
        .bind(&fields.fn_name)
        .bind(&emails_json)
        .bind(&phones_json)
        .bind(&addresses_json)
        .bind(&fields.org)
        .bind(fields.photo.as_deref())
        .execute(&mut *tx)
        .await?;
        changed_result.push(ContactRef {
            account_id: account_id.to_string(),
            uid: fields.uid,
            addressbook_href: addressbook_href.to_string(),
        });
    }

    let mut deleted_result: Vec<ContactRef> = Vec::new();
    for href in deleted {
        let uid: Option<String> = sqlx::query_scalar(
            "SELECT uid FROM cached_contacts WHERE account_id = ? AND addressbook_href = ? AND href = ?",
        )
        .bind(account_id)
        .bind(addressbook_href)
        .bind(&href)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(uid) = uid {
            deleted_result.push(ContactRef {
                account_id: account_id.to_string(),
                uid: uid.clone(),
                addressbook_href: addressbook_href.to_string(),
            });
            sqlx::query(
                "DELETE FROM cached_contacts WHERE account_id = ? AND addressbook_href = ? AND uid = ?",
            )
            .bind(account_id)
            .bind(addressbook_href)
            .bind(&uid)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                "DELETE FROM cached_contacts WHERE account_id = ? AND addressbook_href = ? AND href = ?",
            )
            .bind(account_id)
            .bind(addressbook_href)
            .bind(&href)
            .execute(&mut *tx)
            .await?;
        }
    }

    sqlx::query(
        "UPDATE cached_addressbooks SET ctag = ?, sync_token = ? WHERE account_id = ? AND href = ?",
    )
    .bind(stored_ctag)
    .bind(new_token)
    .bind(account_id)
    .bind(addressbook_href)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(ContactSyncApplyResult {
        changed: changed_result,
        deleted: deleted_result,
    })
}

pub async fn replace_addressbook_contacts(
    pool: &SqlitePool,
    account_id: &str,
    addressbook_href: &str,
    all_raw: Vec<(String, String, String)>,
    new_ctag: Option<&str>,
    new_token: Option<&str>,
) -> anyhow::Result<()> {
    use crate::carddav::parse_vcard;
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM cached_contacts WHERE account_id = ? AND addressbook_href = ?")
        .bind(account_id)
        .bind(addressbook_href)
        .execute(&mut *tx)
        .await?;

    for (href, etag, raw_vcard) in all_raw {
        let fields = parse_vcard(&raw_vcard);
        if fields.uid.is_empty() {
            continue;
        }
        let emails_json =
            serde_json::to_string(&fields.emails).unwrap_or_else(|_| "[]".to_string());
        let phones_json =
            serde_json::to_string(&fields.phones).unwrap_or_else(|_| "[]".to_string());
        let addresses_json =
            serde_json::to_string(&fields.addresses).unwrap_or_else(|_| "[]".to_string());
        sqlx::query(
            "INSERT INTO cached_contacts
                 (account_id, addressbook_href, uid, etag, href, raw_vcard, fn_name, emails, phones, addresses, org, photo)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, addressbook_href, uid) DO UPDATE SET
                 etag = excluded.etag,
                 href = excluded.href,
                 raw_vcard = excluded.raw_vcard,
                 fn_name = excluded.fn_name,
                 emails = excluded.emails,
                 phones = excluded.phones,
                 addresses = excluded.addresses,
                 org = excluded.org,
                 photo = excluded.photo",
        )
        .bind(account_id)
        .bind(addressbook_href)
        .bind(&fields.uid)
        .bind(&etag)
        .bind(&href)
        .bind(&raw_vcard)
        .bind(&fields.fn_name)
        .bind(&emails_json)
        .bind(&phones_json)
        .bind(&addresses_json)
        .bind(&fields.org)
        .bind(fields.photo.as_deref())
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE cached_addressbooks SET ctag = ?, sync_token = ? WHERE account_id = ? AND href = ?",
    )
    .bind(new_ctag)
    .bind(new_token)
    .bind(account_id)
    .bind(addressbook_href)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

// ── Contact push subscriptions ────────────────────────────────────────────────

pub async fn upsert_contact_push_subscription(
    pool: &SqlitePool,
    account_id: &str,
    addressbook_href: &str,
    webhook_token: &str,
    transport: &str,
    expires_at: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO contact_push_subscriptions (account_id, addressbook_href, webhook_token, transport, expires_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(account_id, addressbook_href) DO UPDATE SET
             webhook_token = excluded.webhook_token,
             transport = excluded.transport,
             expires_at = excluded.expires_at",
    )
    .bind(account_id)
    .bind(addressbook_href)
    .bind(webhook_token)
    .bind(transport)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_contact_push_subscription(
    pool: &SqlitePool,
    account_id: &str,
    addressbook_href: &str,
) -> anyhow::Result<Option<(String, i64)>> {
    let row = sqlx::query(
        "SELECT webhook_token, expires_at FROM contact_push_subscriptions WHERE account_id = ? AND addressbook_href = ?",
    )
    .bind(account_id)
    .bind(addressbook_href)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.get(0), r.get(1))))
}

pub async fn get_user_and_addressbook_from_push_token(
    pool: &SqlitePool,
    token: &str,
) -> anyhow::Result<Option<(String, String, String)>> {
    let row = sqlx::query(
        "SELECT ca.user_id, p.account_id, p.addressbook_href
         FROM contact_push_subscriptions p
         JOIN caldav_accounts ca ON ca.account_id = p.account_id
         WHERE p.webhook_token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.get(0), r.get(1), r.get(2))))
}
