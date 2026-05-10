use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_oidc::{ClearSessionFlag, EmptyAdditionalClaims, OidcClaims, OidcRpInitiatedLogout};
use clap::ValueEnum;
use crate::caldav::CalDavClient;
use crate::db;
use crate::db::Db;
use crate::settings::html_escape;
use std::sync::Arc;

#[derive(Clone, Debug, ValueEnum)]
pub enum UserIdClaim {
    Sub,
    PreferredUsername,
    Email,
    Name,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub do_oidc_logout: bool,
    pub oidc_user_id_claim: UserIdClaim,
    pub encryption_key: [u8; 32],
}

pub fn extract_user_id(claims: &OidcClaims<EmptyAdditionalClaims>, claim: &UserIdClaim) -> Option<String> {
    match claim {
        UserIdClaim::Sub => Some(claims.subject().as_str().to_owned()),
        UserIdClaim::PreferredUsername => claims
            .preferred_username()
            .map(|u| u.as_str().to_owned()),
        UserIdClaim::Email => claims.email().map(|e| e.as_str().to_owned()),
        UserIdClaim::Name => claims
            .name()
            .and_then(|n| n.get(None).map(|s| s.as_str().to_owned())),
    }
}

pub fn require_client(
    state: &AppState,
    claims: &OidcClaims<EmptyAdditionalClaims>,
) -> Result<CalDavClient, Response> {
    let user_id = match extract_user_id(claims, &state.oidc_user_id_claim) {
        Some(id) => id,
        None => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Could not determine user ID from OIDC claims").into_response());
        }
    };

    let db = state.db.lock().unwrap();
    let settings = match db::get_settings(&db, &user_id, &state.encryption_key) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Database error: {e}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response());
        }
    };

    match settings {
        Some(s) => CalDavClient::new(s.caldav_base_url, s.caldav_username, s.caldav_password)
            .map_err(|e| {
                eprintln!("CalDAV client error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "CalDAV configuration error").into_response()
            }),
        None => {
            let mut response = (StatusCode::OK, "Redirecting to settings\u{2026}").into_response();
            response.headers_mut().insert("HX-Redirect", "/settings".parse().unwrap());
            Err(response)
        }
    }
}

pub async fn user_info(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
) -> impl IntoResponse {
    let display_name = claims
        .name()
        .and_then(|n| n.get(None).map(|s| s.as_str().to_owned()))
        .or_else(|| claims.preferred_username().map(|u| u.as_str().to_owned()))
        .or_else(|| Some(claims.subject().as_str().to_owned()))
        .unwrap_or_default();

    let has_settings = {
        let user_id = extract_user_id(&claims, &state.oidc_user_id_claim);
        match user_id {
            Some(uid) => {
                let db = state.db.lock().unwrap();
                db::get_settings(&db, &uid, &state.encryption_key).ok().flatten().is_some()
            }
            None => false,
        }
    };

    let settings_link = if has_settings {
        r#" | <a href="/settings">Settings</a>"#
    } else {
        r#" | <a href="/settings">Set up CalDAV</a>"#
    };

    Html(format!(
        r#"<span>{}</span>{} | <a href="/logout">Logout</a>"#,
        html_escape(&display_name),
        settings_link,
    ))
}

pub async fn logout_handler(
    State(state): State<AppState>,
    logout: OidcRpInitiatedLogout,
) -> Response {
    let response = if state.do_oidc_logout {
        match logout.uri() {
            Ok(uri) => {
                let s = uri.to_string();
                Redirect::to(&s).into_response()
            }
            Err(_) => Redirect::to("/").into_response(),
        }
    } else {
        Redirect::to("/").into_response()
    };
    let mut response = response;
    response.extensions_mut().insert(ClearSessionFlag);
    response
}