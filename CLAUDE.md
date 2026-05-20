# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Caley** is a CalDAV calendar viewer. It's a two-package monorepo with independent frontend and backend. Users authenticate (via OIDC or directly with CalDAV credentials), then fetch and view calendar events. Events can be created, edited (including recurring event modifications), and deleted.

## Authentication Modes

Caley supports two auth modes selected at startup based on env vars:

**OIDC mode** — all three of `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` are set:
- `axum-oidc` handles the OIDC flow; `OidcLoginLayer` redirects unauthenticated requests to the provider
- `RequireAuthLayer` checks for `OidcClaims` in request extensions
- User ID extracted from the configured OIDC claim (`OIDC_USER_ID_CLAIM`)
- CalDAV credentials configured separately in the settings page after login

**Local CalDAV login mode** — any OIDC var is absent:
- No external identity provider needed
- Users log in at `/settings/` with CalDAV server URL, username, and password
- User ID derived as `username@hostname` (e.g. `user@dav.myurl.tld`)
- Display name fetched via PROPFIND on the CalDAV principal URL
- Credentials validated then saved to DB and session in one step
- `RequireAuthLayer` checks the session for `user_id`; unauthenticated requests to `/api/` get 401, all other paths redirect to `/settings/`
- Static assets (paths with a file extension, `/assets/*`) always pass through unauthenticated

The `CurrentUser` struct in `oidc.rs` is the unified extractor used by all handlers — it reads from OIDC claims in OIDC mode or from the session in local mode. No handler takes `OidcClaims` directly.

## Architecture

### Backend (`backend/`)
- **Language**: Rust (edition 2024)
- **Framework**: Axum web server
- **Key Dependencies**:
  - `axum-oidc`: OIDC authentication middleware (used only in OIDC mode)
  - `sqlx`: SQLite database (bundled)
  - `ical` + `rrule`: iCalendar parsing and recurrence rules
  - `reqwest`: CalDAV client HTTP requests
  - `aes-gcm` + `hkdf`: Encryption for stored credentials (AES-256-GCM)
  - `dotenvy`: Environment variable loading
  - `tower-sessions`: In-memory session store

**Key Modules**:
- `main.rs`: Server setup, routing, conditional OIDC vs local auth middleware stack
- `search.rs`: `SearchMode` enum (`Default`, `Trigram`, `Stemmed(Algorithm)`). Parsed from `SEARCH_MODE` env var via `FromStr` — `"German"` etc. deserialised into `rust_stemmers::Algorithm` using serde. `normalize_text()` applies Snowball stemming for `Stemmed` mode; passthrough for others. `build_fts_query()` builds the FTS5 MATCH expression (prefix wildcards for default/stemmed, raw terms for trigram). Used in `AppState`, `SyncManager`, and all db insert paths.
- `oidc.rs`: `AuthMode` enum, `CurrentUser` extractor, `AppState`, `RequireAuthLayer`, OIDC-specific handlers (`user_info`, `logout_handler`, `auth_callback_handler`)
- `local_auth.rs`: Local login handler (`handle_login`), logout, `delete_account`, `derive_user_id`
- `caldav.rs`: CalDAV client implementation, iCalendar parsing, event/calendar operations, `get_display_name()`
- `calendar.rs`: HTTP endpoints for calendar/event operations (list, create, edit, delete)
- `db.rs`: SQLite schema and operations (user settings, deactivated calendars, event cache). Credentials encrypted with AES-256-GCM. Includes `delete_user()` for account deletion.
- `settings.rs`: Settings form endpoint, credential validation via test CalDAV connection
- `crypto.rs`: Encryption/decryption using HKDF-SHA256 key derivation from `ENCRYPTION_KEY` env var
- `sync.rs`: Background CalDAV sync manager
- `sse.rs`: Server-sent events for real-time calendar updates

**Database Schema**:
- `user_settings`: Stores per-user CalDAV credentials (encrypted at rest)
- `deactivated_calendars`: Tracks which calendars are hidden per user (encrypted hrefs)
- `cached_calendars`, `cached_events`: Event/calendar cache for fast reads. `cached_events` has `summary_idx`, `description_idx`, `location_idx` columns that hold the FTS-indexed text (stemmed for `de`/`en`, raw otherwise). `events_fts` virtual table indexes these columns.
- `push_subscriptions`: WebDAV Push notification subscriptions
- `app_config`: Key/value store for app-level config (currently stores `search_mode` to detect tokenizer changes on restart)

**Routing**:
- `POST /auth/login` — Local mode login (validates CalDAV credentials, sets session)
- `/auth/callback` — OIDC callback (OIDC mode only)
- `/api/user` — Current user info JSON (`display_name`, `has_settings`, `auth_mode`)
- `/api/calendars` — List calendars
- `/api/calendar/toggle` — Show/hide calendar
- `/api/calendar` — Get month grid for a single calendar
- `/api/events` — Get all events for date range
- `/api/events/stream` — SSE stream for real-time updates
- `/api/settings/form` — Get existing settings JSON
- `/api/settings` — Save CalDAV credentials (validates connection first)
- `/api/event` — Create/update/delete events
- `/api/account` — `DELETE`: delete all user data (and session in local mode)
- `/api/push/webhook` — WebDAV Push webhook (public)
- `/logout` — Logout (OIDC RP-initiated logout in OIDC mode; session flush + redirect in local mode)
- `*` — Fallback: serve frontend static files

### Frontend (`frontend/`)
- **Framework**: Vite + Svelte 5
- **Styling**: Pico CSS + Sass (via `sass-embedded`, not dart-sass)
- **Internationalization**: i18n with English and German translations via `@inlang/paraglide-js`

**Pages** (Vite multi-page build entries):
- `/` (`index.html`) — Main calendar view; mounts `CalendarApp.svelte`
- `/settings/` (`settings/index.html`) — Settings page; mounts `SettingsForm.svelte`

**Components**:
- `CalendarApp.svelte`: Full calendar UI (month grid, week view, event CRUD)
- `SettingsForm.svelte`: Settings/login form — detects auth state on mount via `/api/user`; shows login form (submits to `/auth/login`) when unauthenticated, settings form (submits to `/api/settings`) when authenticated. Also contains the "Delete account" button.

**Dev Server Proxy**: `vite.config.ts` proxies `/api`, `/auth`, and `/logout` to `http://localhost:3000` (backend).

## Development Commands

### Backend
```bash
cd backend
cargo run                    # Start server on :3000 (loads .env if present)
cargo build --release       # Build optimized binary
cargo check                 # Check without building
```

**Environment Variables** (set in `backend/.env` or as env vars):
- `APP_BASE_URL` — Public URL (e.g., `http://localhost:3000` for local dev) **required**
- `ENCRYPTION_KEY` — Passphrase for deriving AES-256-GCM encryption key **required**
- `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` — All required for OIDC mode; omit all three for local CalDAV login mode
- `CALDAV_PORT` — Listen port (default: `3000`)
- `OIDC_LOGOUT` — Enable OIDC RP-initiated logout (default: `true`)
- `OIDC_USER_ID_CLAIM` — OIDC claim for user ID: `sub`, `preferred-username`, `email`, or `name` (default: `sub`)
- `DATABASE_PATH` — SQLite database file (default: `./data/caley.db`)
- `STATIC_DIR` — Frontend dist directory (default: `../frontend/dist`)
- `SYNC_INTERVAL_MINS` — Background sync interval in minutes (default: `5`)
- `SEARCH_MODE` — Full-text search strategy: `default` (unicode-aware prefix matching), `trigram` (SQLite trigram tokenizer, language-agnostic substring matching), or a Snowball algorithm name (e.g. `German`, `English`, `French` — full list: Arabic, Danish, Dutch, English, Finnish, French, German, Greek, Hungarian, Italian, Norwegian, Portuguese, Romanian, Russian, Spanish, Swedish, Tamil, Turkish). Parsed via serde — the string must match the `rust_stemmers::Algorithm` variant name exactly. Changing this rebuilds the FTS index on startup. (default: `default`)

No linter, formatter, or test runner configured. No tests yet.

### Frontend
```bash
cd frontend
npm install                 # Install dependencies
npm run dev                 # Dev server on localhost:5173 (proxies /api, /auth, /logout to backend)
npm run build               # Build static site to dist/
npm run preview             # Preview built site locally
```

No linter, formatter, or test runner configured.

## Docker

**Build**:
```bash
docker build -t caley .
```

**Run (OIDC mode)**:
```bash
docker run -p 3000:3000 \
  -v caley-data:/data \
  -e OIDC_ISSUER=https://auth.example.com \
  -e OIDC_CLIENT_ID=caley \
  -e OIDC_CLIENT_SECRET=secret \
  -e APP_BASE_URL=https://calendar.example.com \
  -e ENCRYPTION_KEY=your-passphrase \
  caley
```

**Run (local CalDAV login mode)**:
```bash
docker run -p 3000:3000 \
  -v caley-data:/data \
  -e APP_BASE_URL=https://calendar.example.com \
  -e ENCRYPTION_KEY=your-passphrase \
  caley
```

Multi-stage build: Node builds frontend, Rust builds backend (musl target), final image is scratch with binary and `/static` directory. Frontend dist is served as fallback when no API route matches.

## Local Development Workflow

### OIDC mode
1. **Start backend**: `cd backend && cargo run` (with OIDC env vars in `backend/.env`)
2. **Start frontend dev server**: `cd frontend && npm run dev`
3. **Open browser**: `http://localhost:5173`
4. **Log in**: Frontend redirects to OIDC provider, then back to `/auth/callback`
5. **Set up CalDAV**: New users redirected to `/settings` after login

### Local CalDAV login mode
1. **Start backend**: `cd backend && cargo run` (only `APP_BASE_URL` and `ENCRYPTION_KEY` needed)
2. **Start frontend dev server**: `cd frontend && npm run dev`
3. **Open browser**: `http://localhost:5173`
4. **Log in**: Navigate to `/settings/`, enter CalDAV server URL, username, and password

## Key Implementation Details

### Auth layer
- `RequireAuthLayer` in `oidc.rs` carries an `AuthMode` and behaves differently per mode
- In local mode it checks `session.get::<String>("user_id")` asynchronously
- Static assets (file extension in path, or `/assets/` prefix) always pass through — otherwise the page's own JS/CSS would be redirected

### CalDAV
- Credentials stored encrypted per-user in SQLite
- CalDAV client fetches calendars (PROPFIND), events (REPORT), supports basic auth
- iCalendar parsing with `ical` crate; recurrence rules via `rrule`
- Events can have recurrence info; editing a recurring event creates an override instance
- All-day events handled differently from timed events (no timezone)
- `get_display_name()` on `CalDavClient` does PROPFIND for `current-user-principal` then `displayname`

### Encryption
- `ENCRYPTION_KEY` env var is hashed with HKDF-SHA256 to derive a 32-byte key
- Credentials encrypted with AES-256-GCM; stored as base64 in database
- Each encrypted field includes a random nonce (not reused)

### Frontend–Backend Communication
- Frontend is a static Vite/Svelte build
- `fetch()` calls to `/api/` endpoints return JSON
- SSE stream (`/api/events/stream`) pushes `events_refetch`, `calendars_changed`, and `event_removed` events
- `SettingsForm.svelte` calls `/api/user` on mount to detect auth mode (401 → login mode)

## Notes

- **No CI/CD configured**: No GitHub Actions, linters, formatters, or test runners
- **No root workspace**: `frontend/` and `backend/` are independent packages; no root `package.json` or Cargo workspace
- **Session storage**: In-memory only; sessions lost on restart. For production, consider persistent session store
- **Reverse proxy required**: App does not handle TLS; recommended to run behind Caddy, nginx, or Traefik
- **Podman compatible**: Works as drop-in replacement for Docker
