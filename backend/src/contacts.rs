use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::carddav::{self, AddressEntry, TypedEntry, build_vcard_preserving, contact_href};
use crate::db;
use crate::oidc::{AppState, CurrentUser};
use crate::sse::SseMessage;

#[derive(Serialize, Clone, utoipa::ToSchema)]
pub struct AddressbookJson {
    pub account_id: String,
    pub href: String,
    pub name: String,
}

#[derive(Serialize, Clone)]
pub struct ContactJson {
    pub account_id: String,
    pub uid: String,
    pub addressbook_href: String,
    pub fn_name: String,
    pub emails: Vec<TypedEntry>,
    pub phones: Vec<TypedEntry>,
    pub addresses: Vec<AddressEntry>,
    pub org: String,
    pub has_photo: bool,
    /// Version of the photo (a hash of it), sent as `v` with the photo URL so the browser may keep the photo
    /// until it changes. None when there is no photo GET /api/contact/photo would serve.
    pub photo_rev: Option<String>,
}

/// Whether GET /api/contact/photo serves this photo: only embedded raster images (`data:image/…`, not SVG).
fn servable_photo(photo: &str) -> bool {
    photo
        .strip_prefix("data:")
        .and_then(|rest| rest.split([';', ',']).next())
        .map(|mime| {
            let mime = mime.to_ascii_lowercase();
            mime.starts_with("image/") && !mime.contains("svg")
        })
        .unwrap_or(false)
}

/// The photo's version: the first 16 hex digits of its SHA-256, when it is served at all.
fn photo_rev(photo: Option<&str>) -> Option<String> {
    use sha2::{Digest, Sha256};
    let photo = photo.filter(|p| servable_photo(p))?;
    let digest = Sha256::digest(photo.as_bytes());
    Some(digest.iter().take(8).map(|b| format!("{b:02x}")).collect())
}

#[derive(Serialize)]
pub struct ContactsResponse {
    pub addressbooks: Vec<AddressbookJson>,
    pub contacts: Vec<ContactJson>,
}

fn parse_typed_entries(json: &str) -> Vec<TypedEntry> {
    serde_json::from_str::<Vec<TypedEntry>>(json).unwrap_or_else(|_| {
        serde_json::from_str::<Vec<String>>(json)
            .unwrap_or_default()
            .into_iter()
            .map(|v| TypedEntry {
                value: v,
                type_label: "other".into(),
            })
            .collect()
    })
}

pub fn cached_to_json_pub(c: &db::CachedContact) -> ContactJson {
    cached_to_json(c)
}

fn cached_to_json(c: &db::CachedContact) -> ContactJson {
    let emails = parse_typed_entries(&c.emails);
    let phones = parse_typed_entries(&c.phones);
    let addresses: Vec<AddressEntry> = serde_json::from_str(&c.addresses).unwrap_or_default();
    ContactJson {
        account_id: c.account_id.clone(),
        uid: c.uid.clone(),
        addressbook_href: c.addressbook_href.clone(),
        fn_name: c.fn_name.clone(),
        emails,
        phones,
        addresses,
        org: c.org.clone(),
        has_photo: c.photo.as_deref().is_some_and(servable_photo),
        photo_rev: photo_rev(c.photo.as_deref()),
    }
}

async fn require_account_client(
    state: &AppState,
    user: &CurrentUser,
    account_id: &str,
) -> Result<crate::caldav::CalDavClient, Response> {
    if state.demo_mode {
        return Ok(crate::caldav::CalDavClient::demo());
    }
    match db::get_caldav_account(&state.db, &user.user_id, account_id, &state.encryption_key).await
    {
        Ok(Some(account)) => crate::caldav::CalDavClient::new(
            account.caldav_base_url,
            account.caldav_username,
            account.caldav_password,
        )
        .map_err(|e| {
            eprintln!("CalDAV client error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "CalDAV configuration error",
            )
                .into_response()
        }),
        Ok(None) => Err((StatusCode::NOT_FOUND, "CalDAV account not found").into_response()),
        Err(e) => {
            eprintln!("[contacts] account lookup error: {e}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response())
        }
    }
}

// GET /api/contacts
pub async fn list_contacts(State(state): State<AppState>, user: CurrentUser) -> impl IntoResponse {
    let addressbooks = match db::get_cached_addressbooks(&state.db, &user.user_id).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[contacts] get_cached_addressbooks error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };
    let contacts = match db::get_contacts(&state.db, &user.user_id).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[contacts] get_contacts error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let resp = ContactsResponse {
        addressbooks: addressbooks
            .into_iter()
            .map(|a| AddressbookJson {
                account_id: a.account_id,
                href: a.href,
                name: a.name,
            })
            .collect(),
        contacts: contacts.iter().map(cached_to_json).collect(),
    };
    Json(resp).into_response()
}

// POST /api/contact
#[derive(Deserialize)]
pub struct SaveContactRequest {
    pub uid: Option<String>,
    pub account_id: String,
    pub addressbook_href: String,
    pub fn_name: String,
    pub emails: Vec<TypedEntry>,
    pub phones: Vec<TypedEntry>,
    pub addresses: Vec<AddressEntry>,
    pub org: String,
}

pub async fn save_contact(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<SaveContactRequest>,
) -> impl IntoResponse {
    let client = match require_account_client(&state, &user, &req.account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let (uid, href, existing_raw, existing_photo, is_new) = if let Some(ref existing_uid) = req.uid
    {
        // Update: look up stored href and raw vCard for non-destructive rebuild
        let stored = match db::get_contact(
            &state.db,
            &req.account_id,
            &req.addressbook_href,
            existing_uid,
        )
        .await
        {
            Ok(Some(c)) => c,
            Ok(None) => return (StatusCode::NOT_FOUND, "Contact not found").into_response(),
            Err(e) => {
                eprintln!("[contacts] get_contact error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
            }
        };
        let h = stored
            .href
            .unwrap_or_else(|| contact_href(&req.addressbook_href, existing_uid));
        (
            existing_uid.clone(),
            h,
            Some(stored.raw_vcard),
            stored.photo,
            false,
        )
    } else {
        // Create: generate new uid and href
        let new_uid = Uuid::new_v4().to_string();
        let h = contact_href(&req.addressbook_href, &new_uid);
        (new_uid, h, None, None, true)
    };

    let raw_vcard = build_vcard_preserving(
        existing_raw.as_deref(),
        &uid,
        &req.fn_name,
        &req.emails,
        &req.phones,
        &req.addresses,
        &req.org,
    );

    let new_etag = match carddav::put_contact(&client, &href, &raw_vcard, is_new).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[contacts] put_contact error: {e}");
            return (StatusCode::BAD_GATEWAY, "Failed to save contact on server").into_response();
        }
    };

    // Update local cache
    let emails_json = serde_json::to_string(&req.emails).unwrap_or_else(|_| "[]".to_string());
    let phones_json = serde_json::to_string(&req.phones).unwrap_or_else(|_| "[]".to_string());
    let addresses_json = serde_json::to_string(&req.addresses).unwrap_or_else(|_| "[]".to_string());
    let cache_etag = new_etag.unwrap_or_default();
    if let Err(e) = db::upsert_contact(
        &state.db,
        &req.account_id,
        &req.addressbook_href,
        &uid,
        &cache_etag,
        &href,
        &raw_vcard,
        &req.fn_name,
        &emails_json,
        &phones_json,
        &addresses_json,
        &req.org,
        existing_photo.as_deref(),
    )
    .await
    {
        eprintln!("[contacts] upsert_contact error: {e}");
    }

    let contact_json = ContactJson {
        account_id: req.account_id.clone(),
        uid: uid.clone(),
        addressbook_href: req.addressbook_href.clone(),
        fn_name: req.fn_name.clone(),
        emails: req.emails.clone(),
        phones: req.phones.clone(),
        addresses: req.addresses.clone(),
        org: req.org.clone(),
        has_photo: existing_photo.as_deref().is_some_and(servable_photo),
        photo_rev: photo_rev(existing_photo.as_deref()),
    };
    let contact_json_str = serde_json::to_string(&contact_json).ok();

    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::ContactUpdated {
            account_id: req.account_id,
            uid,
            addressbook_href: req.addressbook_href,
            contact_json: contact_json_str,
        },
    );

    Json(contact_json).into_response()
}

// DELETE /api/contact
#[derive(Deserialize)]
pub struct DeleteContactRequest {
    pub account_id: String,
    pub uid: String,
    pub addressbook_href: String,
}

pub async fn delete_contact(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<DeleteContactRequest>,
) -> impl IntoResponse {
    let client = match require_account_client(&state, &user, &req.account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let stored =
        match db::get_contact(&state.db, &req.account_id, &req.addressbook_href, &req.uid).await {
            Ok(Some(c)) => c,
            Ok(None) => return (StatusCode::NOT_FOUND, "Contact not found").into_response(),
            Err(e) => {
                eprintln!("[contacts] get_contact error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
            }
        };

    let href = stored
        .href
        .unwrap_or_else(|| contact_href(&req.addressbook_href, &req.uid));

    if let Err(e) = carddav::delete_contact(&client, &href).await {
        eprintln!("[contacts] delete_contact error: {e}");
        return (
            StatusCode::BAD_GATEWAY,
            "Failed to delete contact on server",
        )
            .into_response();
    }

    if let Err(e) =
        db::delete_contact_from_cache(&state.db, &req.account_id, &req.addressbook_href, &req.uid)
            .await
    {
        eprintln!("[contacts] delete_contact_from_cache error: {e}");
    }

    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::ContactRemoved {
            account_id: req.account_id,
            uid: req.uid,
            addressbook_href: req.addressbook_href,
        },
    );

    StatusCode::NO_CONTENT.into_response()
}

// GET /api/contact/photo
#[derive(Deserialize)]
pub struct PhotoQuery {
    pub account_id: String,
    pub uid: String,
    pub addressbook_href: String,
    /// The photo's version (ContactJson.photo_rev): with it the response may be kept for good.
    pub v: Option<String>,
}

pub async fn get_contact_photo(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<PhotoQuery>,
    headers: HeaderMap,
) -> Response {
    match db::get_user_id_for_caldav_account(&state.db, &q.account_id).await {
        Ok(Some(owner)) if owner == user.user_id => {}
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("[contacts] account lookup error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    let contact = match db::get_contact(&state.db, &q.account_id, &q.addressbook_href, &q.uid).await
    {
        Ok(Some(c)) => c,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let photo = match contact.photo {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Remote photo URLs are not redirected to: that would be an open redirect, and
    // auto-loading them leaks the user's IP to an arbitrary host chosen by the
    // (untrusted) contact data. Only embedded data: photos are served.
    if photo.starts_with("http://") || photo.starts_with("https://") {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Parse data URI: data:<mime>;base64,<data>
    if let Some(rest) = photo.strip_prefix("data:") {
        if let Some((meta, b64)) = rest.split_once(',') {
            let mime = meta.split(';').next().unwrap_or("application/octet-stream");
            let mime_lower = mime.to_ascii_lowercase();
            // The MIME comes from untrusted contact data. Restrict to non-SVG
            // images so a text/html or image/svg+xml payload can't render as a
            // same-origin document if the URL is opened directly.
            if !mime_lower.starts_with("image/") || mime_lower.contains("svg") {
                return StatusCode::NOT_FOUND.into_response();
            }
            if let Ok(bytes) = BASE64_STANDARD.decode(b64.trim()) {
                // The URL names the version (`v` = photo_rev): a changed photo gets another URL, so the
                // browser keeps this one. Without it (or with an old one), revalidate by ETag.
                let rev = photo_rev(Some(&photo)).unwrap_or_default();
                let etag = format!("\"{rev}\"");
                let cache = if q.v.as_deref() == Some(rev.as_str()) {
                    "private, max-age=31536000, immutable"
                } else {
                    "private, no-cache"
                };
                let fresh = headers
                    .get(header::IF_NONE_MATCH)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.split(',').any(|t| t.trim() == etag));
                let common = [
                    (header::CACHE_CONTROL, cache.to_string()),
                    (header::ETAG, etag),
                    (
                        header::HeaderName::from_static("x-content-type-options"),
                        "nosniff".to_string(),
                    ),
                ];
                if fresh {
                    return (StatusCode::NOT_MODIFIED, common).into_response();
                }
                return (
                    common,
                    [(header::CONTENT_TYPE, mime.to_string())],
                    bytes,
                )
                    .into_response();
            }
        }
    }

    StatusCode::NOT_FOUND.into_response()
}

// GET /api/contact/vcf
pub async fn get_contact_vcf(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<PhotoQuery>,
) -> Response {
    match db::get_user_id_for_caldav_account(&state.db, &q.account_id).await {
        Ok(Some(owner)) if owner == user.user_id => {}
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("[contacts] account lookup error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    let contact = match db::get_contact(&state.db, &q.account_id, &q.addressbook_href, &q.uid).await
    {
        Ok(Some(c)) => c,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [(header::CONTENT_TYPE, "text/vcard; charset=utf-8")],
        contact.raw_vcard,
    )
        .into_response()
}


/// This module's routes that need a signed-in user, relative to `/api` (api.rs). Not yet described by OpenAPI: plain routes.
#[allow(unused_imports)]
pub fn routes() -> utoipa_axum::router::OpenApiRouter<crate::oidc::AppState> {
    use axum::routing::{delete, get, patch, post, put};
    utoipa_axum::router::OpenApiRouter::new()
        .route("/contacts", get(list_contacts))
        .route("/contact", post(save_contact).delete(delete_contact))
        .route("/contact/photo", get(get_contact_photo))
        .route("/contact/vcf", get(get_contact_vcf))
}

#[cfg(test)]
mod photo_tests {
    use super::*;

    #[test]
    fn only_embedded_raster_images_are_served() {
        assert!(servable_photo("data:image/jpeg;base64,AAAA"));
        assert!(servable_photo("data:IMAGE/PNG;base64,AAAA"));
        assert!(!servable_photo("data:image/svg+xml;base64,AAAA"));
        assert!(!servable_photo("data:text/html;base64,AAAA"));
        assert!(!servable_photo("https://example.com/a.jpg"));
    }

    #[test]
    fn photo_rev_changes_with_the_photo() {
        let a = photo_rev(Some("data:image/png;base64,AAAA")).unwrap();
        let b = photo_rev(Some("data:image/png;base64,AAAB")).unwrap();
        assert_eq!(a.len(), 16);
        assert_ne!(a, b);
        assert_eq!(photo_rev(Some("https://example.com/a.jpg")), None);
        assert_eq!(photo_rev(None), None);
    }
}
