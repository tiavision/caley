use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Redirect, Response},
};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::carddav::{self, build_vcard_preserving, contact_href, AddressEntry, TypedEntry};
use crate::db;
use crate::oidc::{require_client, AppState, CurrentUser};
use crate::sse::SseMessage;

#[derive(Serialize, Clone)]
pub struct AddressbookJson {
    pub href: String,
    pub name: String,
}

#[derive(Serialize, Clone)]
pub struct ContactJson {
    pub uid: String,
    pub addressbook_href: String,
    pub fn_name: String,
    pub emails: Vec<TypedEntry>,
    pub phones: Vec<TypedEntry>,
    pub addresses: Vec<AddressEntry>,
    pub org: String,
    pub has_photo: bool,
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
            .map(|v| TypedEntry { value: v, type_label: "other".into() })
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
        uid: c.uid.clone(),
        addressbook_href: c.addressbook_href.clone(),
        fn_name: c.fn_name.clone(),
        emails,
        phones,
        addresses,
        org: c.org.clone(),
        has_photo: c.photo.is_some(),
    }
}

// GET /api/contacts
pub async fn list_contacts(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
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
            .map(|a| AddressbookJson { href: a.href, name: a.name })
            .collect(),
        contacts: contacts.iter().map(cached_to_json).collect(),
    };
    Json(resp).into_response()
}

// POST /api/contact
#[derive(Deserialize)]
pub struct SaveContactRequest {
    pub uid: Option<String>,
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
    let client = match require_client(&state, &user).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let (uid, href, existing_raw, existing_photo, is_new) = if let Some(ref existing_uid) = req.uid {
        // Update: look up stored href and raw vCard for non-destructive rebuild
        let stored = match db::get_contact(&state.db, &user.user_id, &req.addressbook_href, existing_uid).await {
            Ok(Some(c)) => c,
            Ok(None) => return (StatusCode::NOT_FOUND, "Contact not found").into_response(),
            Err(e) => {
                eprintln!("[contacts] get_contact error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
            }
        };
        let h = stored.href.unwrap_or_else(|| contact_href(&req.addressbook_href, existing_uid));
        (existing_uid.clone(), h, Some(stored.raw_vcard), stored.photo, false)
    } else {
        // Create: generate new uid and href
        let new_uid = Uuid::new_v4().to_string();
        let h = contact_href(&req.addressbook_href, &new_uid);
        (new_uid, h, None, None, true)
    };

    let raw_vcard = build_vcard_preserving(
        existing_raw.as_deref(),
        &uid, &req.fn_name, &req.emails, &req.phones, &req.addresses, &req.org,
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
        &user.user_id,
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
    ).await {
        eprintln!("[contacts] upsert_contact error: {e}");
    }

    let contact_json = ContactJson {
        uid: uid.clone(),
        addressbook_href: req.addressbook_href.clone(),
        fn_name: req.fn_name.clone(),
        emails: req.emails.clone(),
        phones: req.phones.clone(),
        addresses: req.addresses.clone(),
        org: req.org.clone(),
        has_photo: existing_photo.is_some(),
    };
    let contact_json_str = serde_json::to_string(&contact_json).ok();

    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::ContactUpdated {
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
    pub uid: String,
    pub addressbook_href: String,
}

pub async fn delete_contact(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<DeleteContactRequest>,
) -> impl IntoResponse {
    let client = match require_client(&state, &user).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let stored = match db::get_contact(&state.db, &user.user_id, &req.addressbook_href, &req.uid).await {
        Ok(Some(c)) => c,
        Ok(None) => return (StatusCode::NOT_FOUND, "Contact not found").into_response(),
        Err(e) => {
            eprintln!("[contacts] get_contact error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let href = stored.href.unwrap_or_else(|| contact_href(&req.addressbook_href, &req.uid));

    if let Err(e) = carddav::delete_contact(&client, &href).await {
        eprintln!("[contacts] delete_contact error: {e}");
        return (StatusCode::BAD_GATEWAY, "Failed to delete contact on server").into_response();
    }

    if let Err(e) = db::delete_contact_from_cache(&state.db, &user.user_id, &req.addressbook_href, &req.uid).await {
        eprintln!("[contacts] delete_contact_from_cache error: {e}");
    }

    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::ContactRemoved {
            uid: req.uid,
            addressbook_href: req.addressbook_href,
        },
    );

    StatusCode::NO_CONTENT.into_response()
}

// GET /api/contact/photo
#[derive(Deserialize)]
pub struct PhotoQuery {
    pub uid: String,
    pub addressbook_href: String,
}

pub async fn get_contact_photo(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<PhotoQuery>,
) -> Response {
    let contact = match db::get_contact(&state.db, &user.user_id, &q.addressbook_href, &q.uid).await {
        Ok(Some(c)) => c,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let photo = match contact.photo {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    if photo.starts_with("http://") || photo.starts_with("https://") {
        return Redirect::temporary(&photo).into_response();
    }

    // Parse data URI: data:<mime>;base64,<data>
    if let Some(rest) = photo.strip_prefix("data:") {
        if let Some((meta, b64)) = rest.split_once(',') {
            let mime = meta.split(';').next().unwrap_or("application/octet-stream");
            if let Ok(bytes) = BASE64_STANDARD.decode(b64.trim()) {
                return (
                    [(header::CONTENT_TYPE, mime.to_string())],
                    bytes,
                ).into_response();
            }
        }
    }

    StatusCode::NOT_FOUND.into_response()
}
