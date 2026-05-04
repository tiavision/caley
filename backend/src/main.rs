mod caldav;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use caldav::CalDavClient;
use clap::Parser;
use dotenvy::dotenv;
use serde::Deserialize;

#[derive(clap::Parser, Debug)]
struct Cli {
    #[arg(env = "CALDAV_BASE_URL")]
    base_url: String,
    #[arg(env = "CALDAV_USERNAME")]
    username: String,
    #[arg(env = "CALDAV_PASSWORD")]
    password: String,
    #[arg(env = "CALDAV_PORT", default_value = "3000")]
    port: u16,
}

#[derive(Clone)]
struct AppState {
    client: CalDavClient,
}

#[derive(Deserialize)]
struct EventsQuery {
    href: String,
}

async fn list_calendars(State(state): State<AppState>) -> impl IntoResponse {
    match state.client.calendars().await {
        Ok(calendars) => Json(calendars).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn list_events(State(state): State<AppState>, Query(params): Query<EventsQuery>) -> impl IntoResponse {
    match state.client.events(&params.href).await {
        Ok(events) => Json(events).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    let client = CalDavClient::new(args.base_url, args.username, args.password)?;
    let state = AppState { client };

    let app = Router::new()
        .route("/calendars", get(list_calendars))
        .route("/events", get(list_events))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}