//! The JSON API under `/api`, described by OpenAPI: every route is registered from its
//! `#[utoipa::path]` annotation (`utoipa_axum::routes!`), so the router and the spec cannot disagree.
//! The spec is `frontend/openapi.json`, which the frontend build reads (`sinter({ apis })`): every
//! resource and action path, query parameter, request body and response type of the pages is checked
//! against it. The test below fails when the file is stale; `UPDATE_OPENAPI=1 cargo test openapi`
//! writes it.
//!
//! Paths in the annotations are relative to `/api` (`path = "/calendars"`), the spec's server.

use utoipa::openapi::OpenApi as Spec;
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;

use crate::oidc::AppState;

#[derive(OpenApi)]
#[openapi(
    info(title = "Caley", description = "Caley's JSON API: calendars, contacts, mail and OpenPGP keys."),
    servers((url = "/api")),
    components(schemas(ApiError, StatusOk, Binary))
)]
struct ApiDoc;

/// An error the API explains: `{"error": "..."}` (4xx; a 5xx is plain text).
#[derive(serde::Serialize, ToSchema)]
pub struct ApiError {
    pub error: String,
}

impl ApiError {
    pub fn new(error: impl Into<String>) -> Self {
        ApiError {
            error: error.into(),
        }
    }
}

/// A write that has nothing else to say: `{"status": "ok"}`.
#[derive(serde::Serialize, ToSchema)]
pub struct StatusOk {
    /// always `ok`
    pub status: &'static str,
}

/// Raw bytes: a binary response (an image, a file), `type: string, format: binary`.
/// (`Vec<u8>` would be described as an array of integers.)
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct Binary(#[allow(dead_code)] Vec<u8>);

impl StatusOk {
    pub const OK: StatusOk = StatusOk { status: "ok" };
}

/// The routes that need a signed-in user (`RequireAuthLayer`), relative to `/api`.
pub fn private() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(crate::calendar::routes())
        .merge(crate::sse::routes())
        .merge(crate::oidc::routes())
        .merge(crate::settings::routes())
        .merge(crate::contacts::routes())
        .merge(crate::local_auth::routes())
        .merge(crate::email::routes())
        .merge(crate::pgp::routes())
}

/// The routes anyone may call, relative to `/api`.
pub fn public() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(crate::sse::public_routes())
        .merge(crate::email::public_routes())
        .merge(crate::oidc::public_routes())
}

/// The spec of every route under `/api`.
pub fn spec() -> Spec {
    let mut doc = ApiDoc::openapi();
    doc.merge(private().into_openapi());
    doc.merge(public().into_openapi());
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/openapi.json");

    #[test]
    fn openapi_spec_is_current() {
        let json = spec().to_pretty_json().unwrap() + "\n";
        if std::env::var_os("UPDATE_OPENAPI").is_some() {
            std::fs::write(FILE, &json).unwrap();
            return;
        }
        let current = std::fs::read_to_string(FILE).unwrap_or_default();
        assert!(
            current == json,
            "frontend/openapi.json is stale: run `UPDATE_OPENAPI=1 cargo test openapi` in backend/"
        );
    }
}
