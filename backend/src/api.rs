//! The JSON API under `/api`, described by OpenAPI: every route is registered from its
//! `#[utoipa::path]` annotation (`utoipa_axum::routes!`), so the router and the spec cannot disagree.
//! The spec is `frontend/openapi.json`, which the frontend build reads (`sinter({ apis })`): every
//! resource and action path, query parameter, request body and response type of the pages is checked
//! against it. The test below fails when the file is stale; `UPDATE_OPENAPI=1 cargo test openapi`
//! writes it. `caley --openapi` prints the spec (the Docker image builds the frontend against it).
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

// ── Request helpers ──────────────────────────────────────────────────────────
// Handlers take their requests as typed extractors (`axum_extra::extract::Query` / `Form`, which
// read repeated keys into a `Vec`; `axum_typed_multipart::TypedMultipart`), and the same structs
// are the spec's parameters and bodies.

/// `#[serde(default, deserialize_with = "crate::api::empty_as_none")]`: an optional value parsed
/// with `FromStr`, where an empty string (a form field left empty) is `None`.
pub fn empty_as_none<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    use serde::Deserialize as _;
    match Option::<String>::deserialize(de)?.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => s.parse().map(Some).map_err(serde::de::Error::custom),
    }
}

/// A multipart field holding JSON (`#[schema(value_type = String)]` on the field).
pub struct JsonField<T>(pub T);

#[axum_typed_multipart::async_trait]
impl<T: serde::de::DeserializeOwned> axum_typed_multipart::TryFromChunks for JsonField<T> {
    async fn try_from_chunks(
        chunks: impl futures_util::Stream<
            Item = Result<bytes::Bytes, axum_typed_multipart::TypedMultipartError>,
        > + Send
        + Sync
        + Unpin,
        metadata: axum_typed_multipart::FieldMetadata,
    ) -> Result<Self, axum_typed_multipart::TypedMultipartError> {
        let field_name = metadata.name.clone().unwrap_or_default();
        let bytes = bytes::Bytes::try_from_chunks(chunks, metadata).await?;
        serde_json::from_slice(&bytes).map(JsonField).map_err(|e| {
            axum_typed_multipart::TypedMultipartError::WrongFieldType {
                field_name,
                wanted_type: "JSON".into(),
                source: e.into(),
            }
        })
    }
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

/// The spec of every route under `/api` (`caley --openapi` prints it).
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
