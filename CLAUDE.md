# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Caley** is a CalDAV calendar viewer. It's a two-package monorepo with independent frontend and backend. Users authenticate (via OIDC or directly with CalDAV credentials), then fetch and view calendar events. Events can be created, edited (including recurring event modifications), and deleted.

## Authentication

CalDAV-credential login is **always** available; OIDC login is **additive** — offered alongside it when configured. Both can authenticate users at the same time. The unified login page lives at `/` (the index): a CalDAV form (server URL, username, password) plus, when OIDC is configured, a "sign in via provider" button.

**CalDAV-credential login** (always on):
- User submits credentials to `POST /auth/login`; they are validated against the CalDAV server (PROPFIND), then saved to the DB and a session is created
- User ID derived as `username@hostname` (e.g. `user@dav.myurl.tld`); display name fetched via PROPFIND on the principal URL
- Authenticated state = a `user_id` key in the session

**OIDC login** (when `OIDC_ISSUER` + `OIDC_CLIENT_ID` + `OIDC_CLIENT_SECRET` are all set):
- The login button links to `/auth/oidc`, which is wrapped by `OidcLoginLayer` and redirects to the provider; on return the code is exchanged on that same route, then the user is sent to `/calendar/`
- **The IdP redirect URI must be `{APP_BASE_URL}/auth/oidc`** (not `/auth/callback`, which no longer exists)
- `OidcAuthLayer` wraps the whole app (a safe no-op for CalDAV-only users) so `OidcClaims` are populated when an OIDC session exists
- User ID extracted from the configured OIDC claim (`OIDC_USER_ID_CLAIM`); CalDAV credentials configured separately in settings after login

**Gating** (`RequireAuthLayer` in `oidc.rs`): public paths (`/`, `/auth/*`, `/logout`, `/api/auth/config`, static assets) always pass; otherwise authenticated = (OIDC claims present) OR (session `user_id` present). Unauthenticated `/api/*` requests get 401 JSON; all other unauthenticated paths redirect to `/`.

`/logout` is unified: it does RP-initiated logout when an OIDC session is present (extracted via `Option<OidcRpInitiatedLogout>`), otherwise just flushes the session.

The `CurrentUser` struct in `oidc.rs` is the unified extractor used by all handlers — it tries OIDC claims first, then the session. No handler takes `OidcClaims` directly. `GET /api/auth/config` is a public endpoint reporting `{ oidc_enabled }` so the login page knows whether to show the OIDC button.

## Architecture

### Backend (`backend/`)
- **Language**: Rust (edition 2024)
- **Framework**: Axum web server
- **Key Dependencies**:
  - `axum-oidc`: OIDC authentication middleware (active when OIDC is configured)
  - `sqlx`: SQLite database (bundled)
  - `ical` + `rrule`: iCalendar parsing and recurrence rules
  - `reqwest`: CalDAV client HTTP requests
  - `aes-gcm` + `hkdf`: Encryption for stored credentials (AES-256-GCM)
  - `dotenvy`: Environment variable loading
  - `tower-sessions`: In-memory session store

**Key Modules**:
- `main.rs`: Server setup, routing, unified auth middleware stack (CalDAV login always wired; OIDC layers added when configured)
- `search.rs`: `SearchMode` enum (`Default`, `Trigram`, `Stemmed(Algorithm)`). Parsed from `SEARCH_MODE` env var via `FromStr` — `"German"` etc. deserialised into `rust_stemmers::Algorithm` using serde. `normalize_text()` applies Snowball stemming for `Stemmed` mode; passthrough for others. `build_fts_query()` builds the FTS5 MATCH expression (prefix wildcards for default/stemmed, raw terms for trigram). Used in `AppState`, `SyncManager`, and all db insert paths.
- `oidc.rs`: `CurrentUser` extractor (claims-then-session), `AppState` (`oidc_enabled: bool`), `RequireAuthLayer`, handlers (`user_info`, `auth_config`, unified `logout_handler`, `oidc_post_login`)
- `local_auth.rs`: CalDAV login handler (`handle_login`), `delete_account`, `derive_user_id`
- `caldav.rs`: CalDAV client implementation, iCalendar parsing, event/calendar operations, `get_display_name()`
- `calendar.rs`: HTTP endpoints for calendar/event operations (list, create, edit, delete)
- `db.rs`: SQLite schema and operations (user identity record, CalDAV accounts, deactivated calendars, event cache). CalDAV credentials encrypted with AES-256-GCM. `ensure_user()` creates the identity row on login; `get_settings()`/`upsert_settings()` operate on the user's active CalDAV account; `delete_user()` for account deletion.
- `settings.rs`: Settings form endpoint, credential validation via test CalDAV connection
- `crypto.rs`: Encryption/decryption using HKDF-SHA256 key derivation from `ENCRYPTION_KEY` env var
- `sync.rs`: Background CalDAV sync manager
- `sse.rs`: Server-sent events for real-time calendar updates

**Database Schema**:
- `user_settings`: Per-user identity record (`user_id`, `display_name`). Exists for every authenticated user, even with no CalDAV account configured; referenced by the cache/contacts/email tables.
- `caldav_accounts`: Per-user CalDAV credentials (encrypted at rest), keyed by `account_id` with a `user_id` foreign column. Modelled to hold multiple accounts per user; downstream sync/cache currently use the single active account (`get_settings` returns the first by `sort_order`).
- `deactivated_calendars`: Tracks which calendars are hidden per user (encrypted hrefs)
- `cached_calendars`, `cached_events`: Event/calendar cache for fast reads. `cached_events` has `summary_idx`, `description_idx`, `location_idx` columns that hold the FTS-indexed text (stemmed for `de`/`en`, raw otherwise). `events_fts` virtual table indexes these columns.
- `push_subscriptions`: WebDAV Push notification subscriptions
- `app_config`: Key/value store for app-level config (currently stores `search_mode` to detect tokenizer changes on restart)

**Routing**:
- `POST /auth/login` — CalDAV-credential login (validates credentials, sets session) — always available
- `/auth/oidc` — OIDC login trigger + callback (only when OIDC configured); IdP redirect URI must point here
- `/api/auth/config` — Public; reports `{ oidc_enabled }` for the login page
- `/api/user` — Current user info JSON (`display_name`, `has_settings`, `has_caldav`, `oidc_enabled`)
- `/api/calendars` — List calendars
- `/api/calendar/toggle` — Show/hide calendar
- `/api/calendar` — Get month grid for a single calendar
- `/api/events` — Get all events for date range
- `/api/events/stream` — SSE stream for real-time updates
- `/api/settings/form` — Get existing settings JSON
- `/api/settings` — Save CalDAV credentials (validates connection first)
- `/api/event` — Create/update/delete events
- `/api/account` — `DELETE`: delete all user data, flush session, redirect to `/`
- `/api/push/webhook` — WebDAV Push webhook (public)
- `/logout` — Unified logout (OIDC RP-initiated logout when an OIDC session is present, otherwise session flush + redirect to `/`)
- `*` — Fallback: serve frontend static files

### Frontend (`frontend/`)
- **Framework**: Vite + Svelte 5
- **Styling**: Pico CSS + Sass (via `sass-embedded`, not dart-sass)
- **Internationalization**: i18n with English and German translations via `@inlang/paraglide-js`

**Pages** (Vite multi-page build entries):
- `/` (`index.html`) — Login page; mounts `LoginPage.svelte`
- `/calendar/` (`calendar/index.html`) — Main calendar view; mounts `CalendarApp.svelte`
- `/settings/` (`settings/index.html`) — Settings page (authenticated only); mounts `SettingsForm.svelte`

**Components**:
- `LoginPage.svelte`: Login form (submits CalDAV credentials to `/auth/login`) plus, when `/api/auth/config` reports `oidc_enabled`, a button linking to `/auth/oidc`. Redirects to `/calendar/` if already authenticated.
- `CalendarApp.svelte`: Full calendar UI (month grid, week view, event CRUD)
- `SettingsForm.svelte`: Authenticated settings page — CalDAV connection form (submits to `/api/settings`), calendars tab, language selector, and the "Delete account" button. No longer handles login.

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
- `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` — All three required to enable OIDC login (additive; CalDAV-credential login is always available). Omit them to offer CalDAV login only. When set, register `{APP_BASE_URL}/auth/oidc` as the IdP redirect URI.
- `CALDAV_PORT` — Listen port (default: `3000`)
- `OIDC_LOGOUT` — Enable OIDC RP-initiated logout (default: `true`)
- `OIDC_USER_ID_CLAIM` — OIDC claim for user ID: `sub`, `preferred-username`, `email`, or `name` (default: `sub`)
- `DATABASE_PATH` — SQLite database file (default: `./data/caley.db`)
- `STATIC_DIR` — Frontend dist directory (default: `../frontend/dist`)
- `SYNC_INTERVAL_MINS` — Background sync interval in minutes (default: `5`)
- `MAP_URL` — URL template for the map fallback link on location fields. Use `{q}` as the placeholder for the URL-encoded query (default: `https://www.openstreetmap.org/search?query={q}`)
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

**Run (CalDAV login + OIDC)** — register `https://calendar.example.com/auth/oidc` as the IdP redirect URI:
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

**Run (CalDAV login only)** — omit the OIDC vars:
```bash
docker run -p 3000:3000 \
  -v caley-data:/data \
  -e APP_BASE_URL=https://calendar.example.com \
  -e ENCRYPTION_KEY=your-passphrase \
  caley
```

Multi-stage build: Node builds frontend, Rust builds backend (musl target), final image is scratch with binary and `/static` directory. Frontend dist is served as fallback when no API route matches.

## Local Development Workflow

1. **Start backend**: `cd backend && cargo run` (`APP_BASE_URL` and `ENCRYPTION_KEY` required; add the OIDC vars to also offer OIDC login)
2. **Start frontend dev server**: `cd frontend && npm run dev`
3. **Open browser**: `http://localhost:5173`
4. **Log in** at `/`: enter CalDAV server URL, username, and password, or (if configured) click the OIDC button
5. **Set up CalDAV** (OIDC users): configure a CalDAV connection in `/settings/` after login

## Key Implementation Details

### Auth layer
- `RequireAuthLayer` in `oidc.rs` lets the public allow-list (`/`, `/auth/*`, `/logout`, `/api/auth/config`, static assets) through, then requires either OIDC claims (populated by the broad `OidcAuthLayer`) or a session `user_id`
- `OidcAuthLayer` wraps the whole app and is a safe no-op when there is no OIDC session; `OidcLoginLayer` wraps only `/auth/oidc` so it doesn't auto-redirect every request to the IdP
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
