use askama::Template;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_oidc::{EmptyAdditionalClaims, OidcClaims};
use crate::db::UserSettings;

use crate::oidc::{extract_user_id, AppState};

#[derive(Template)]
#[template(path = "settings.html", whitespace = "minimize")]
struct SettingsTemplate {
    base_url: String,
    username: String,
    password: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct SettingsForm {
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub async fn settings_page(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
) -> Response {
    let user_id = match extract_user_id(&claims, &state.oidc_user_id_claim) {
        Some(id) => id,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Could not determine user ID").into_response();
        }
    };

    let db = state.db.lock().unwrap();
    let existing = match crate::db::get_settings(&db, &user_id, &state.encryption_key) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Database error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let (base_url, username, password) = match existing {
        Some(ref s) => (s.caldav_base_url.clone(), s.caldav_username.clone(), s.caldav_password.clone()),
        None => (String::new(), String::new(), String::new()),
    };

    let tpl = SettingsTemplate {
        base_url,
        username,
        password,
    };
    match tpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn save_settings(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
    Form(form): Form<SettingsForm>,
) -> Response {
    let user_id = match extract_user_id(&claims, &state.oidc_user_id_claim) {
        Some(id) => id,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Could not determine user ID").into_response();
        }
    };

    let settings = UserSettings {
        caldav_base_url: form.caldav_base_url,
        caldav_username: form.caldav_username,
        caldav_password: form.caldav_password,
    };

    let client = match crate::caldav::CalDavClient::new(
        settings.caldav_base_url.clone(),
        settings.caldav_username.clone(),
        settings.caldav_password.clone(),
    ) {
        Ok(c) => c,
        Err(e) => {
            return format!(
                r#"<div class="error-msg">Connection failed: {}</div>"#,
                html_escape(&e.to_string()),
            )
            .into_response();
        }
    };

    match client.calendars().await {
        Ok(_) => {}
        Err(e) => {
            return format!(
                r#"<div class="error-msg">Connection test failed: {}</div>"#,
                html_escape(&e.to_string()),
            )
            .into_response();
        }
    }

    let db = state.db.lock().unwrap();
    if let Err(e) = crate::db::upsert_settings(&db, &user_id, &settings, &state.encryption_key) {
        eprintln!("Database error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save settings").into_response();
    }

    r#"<div class="success-msg">Settings saved! Redirecting...</div><script>setTimeout(function(){ window.location.href='/'; }, 500);</script>"#.into_response()
}