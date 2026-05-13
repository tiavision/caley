# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Caley** is a CalDAV calendar viewer with OIDC authentication. It's a two-package monorepo with independent frontend and backend. Users authenticate via OIDC, then configure CalDAV credentials to fetch and view calendar events. Events can be created, edited (including recurring event modifications), and deleted.

## Architecture

### Backend (`backend/`)
- **Language**: Rust (edition 2024)
- **Framework**: Axum web server with Askama HTML templates
- **Key Dependencies**:
  - `axum-oidc`: OIDC authentication middleware
  - `rusqlite`: SQLite database (bundled)
  - `ical` + `rrule`: iCalendar parsing and recurrence rules
  - `reqwest`: CalDAV client HTTP requests
  - `aes-gcm` + `hkdf`: Encryption for stored credentials (AES-256-GCM)
  - `dotenvy`: Environment variable loading
  - `tower-sessions`: In-memory session store

**Key Modules**:
- `main.rs`: Server setup, routing, middleware layers (OIDC, sessions)
- `oidc.rs`: OIDC authentication flow, user info endpoint, custom auth layer (`RequireAuthLayer`)
- `caldav.rs`: CalDAV client implementation, iCalendar parsing, event/calendar operations
- `calendar.rs`: HTTP endpoints for calendar/event operations (list, create, edit, delete), HTML template rendering
- `db.rs`: SQLite schema and operations (user settings, deactivated calendars). Credentials encrypted with AES-256-GCM
- `settings.rs`: Settings form endpoint, credential validation via test CalDAV connection
- `crypto.rs`: Encryption/decryption using HKDF-SHA256 key derivation from `ENCRYPTION_KEY` env var

**Database Schema**:
- `user_settings`: Stores per-user CalDAV credentials (encrypted at rest)
- `deactivated_calendars`: Tracks which calendars are hidden per user (encrypted hrefs)

**Routing**:
- `/auth/callback` — OIDC callback
- `/api/user` — Current user info (HTMX)
- `/api/calendars` — List calendars (HTMX)
- `/api/calendar/toggle` — Show/hide calendar (HTMX)
- `/api/calendar` — Get events for a single calendar
- `/api/events` — Get all events for date range
- `/api/settings/form` — Settings form (HTMX)
- `/api/settings` — Save CalDAV credentials (validates connection first)
- `/api/event` — Create/update/delete events
- `/logout` — OIDC logout
- `*` — Fallback to serve frontend static files

All endpoints except `/auth/callback` require OIDC authentication via `RequireAuthLayer`. Unauthenticated requests redirect to `/auth/callback`.

### Frontend (`frontend/`)
- **Framework**: Astro 6 static site generator
- **Styling**: Pico CSS + Sass (via `sass-embedded`, not dart-sass)
- **Interactivity**: HTMX for dynamic calendar navigation, event creation/editing, user info
- **Internationalization**: i18n with English and German translations

**Pages**:
- `/` (index.astro): Main calendar view (grid layout on desktop, stacked on mobile)
  - Calendar picker component (date navigation)
  - Calendars list component (toggle visibility)
  - Weekly events component (displays events)
- `/settings` (settings.astro): CalDAV credential configuration page

**Components**:
- `Calendar.astro`: Date picker with prev/next navigation
- Weekly.astro`: Displays events for the current week
- `Calendars.astro`: List of available calendars with toggle checkboxes
- `Welcome.astro`: Placeholder (can be removed)

**Layout**:
- `Layout.astro`: Main template with Pico CSS navbar, HTMX script, i18n initialization, user info fetched via HTMX

**Dev Server Proxy**: `astro.config.mjs` proxies `/api` and `/logout` to `http://localhost:3000` (backend).

## Development Commands

### Backend
```bash
cd backend
cargo run                    # Start server on :3000 (loads .env if present)
cargo build --release       # Build optimized binary
cargo check                 # Check without building
```

**Environment Variables** (required for local dev, set in `backend/.env` or as env vars):
- `OIDC_ISSUER` — OIDC provider issuer URL
- `OIDC_CLIENT_ID` — OIDC client ID
- `OIDC_CLIENT_SECRET` — OIDC client secret
- `APP_BASE_URL` — Public URL (e.g., `http://localhost:3000` for local dev)
- `ENCRYPTION_KEY` — Passphrase for deriving AES-256-GCM encryption key
- `CALDAV_PORT` — Listen port (default: `3000`)
- `OIDC_LOGOUT` — Enable OIDC logout (default: `true`)
- `OIDC_USER_ID_CLAIM` — OIDC claim for user ID: `sub`, `preferred-username`, `email`, or `name` (default: `sub`)
- `DATABASE_PATH` — SQLite database file (default: `./data/caley.db`)
- `STATIC_DIR` — Frontend dist directory (default: `../frontend/dist`)

No linter, formatter, or test runner configured. No tests yet.

### Frontend
```bash
cd frontend
npm install                 # Install dependencies
npm run dev                 # Dev server on localhost:4321 (proxies /api to backend)
npm run build               # Build static site to dist/
npm run preview             # Preview built site locally
```

No linter, formatter, or test runner configured.

## Docker

**Build**:
```bash
docker build -t caley .
```

**Run**:
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

Multi-stage build: Node builds frontend, Rust builds backend (musl target), final image is scratch with binary and `/static` directory. Frontend dist is served as fallback when no API route matches.

## Local Development Workflow

1. **Start backend**: `cd backend && cargo run`
2. **Start frontend dev server** (in another terminal): `cd frontend && npm run dev`
3. **Open browser**: `http://localhost:4321`
4. **Configure OIDC**: Use an OIDC provider (Pocket ID, Keycloak, Authelia, Authentik). Set env vars in `backend/.env`.
5. **Log in**: Frontend redirects to OIDC provider, then back to `/auth/callback`
6. **Set up CalDAV**: New users redirected to `/settings` after login. Enter CalDAV base URL, username, password. Connection validated before saving.

## Key Implementation Details

### OIDC Flow
- `axum-oidc` discovers OIDC metadata on startup
- `OidcAuthLayer` validates tokens; `OidcLoginLayer` redirects unauthenticated requests to provider
- `RequireAuthLayer` ensures all endpoints (except `/auth/callback`) require auth; redirects to `/auth/callback` if missing
- Sessions stored in memory (`MemoryStore`); lost on restart
- User ID extracted from OIDC claim (configurable via `OIDC_USER_ID_CLAIM`)

### CalDAV
- Credentials stored encrypted per-user in SQLite
- CalDAV client fetches calendars (PROPFIND), events (REPORT), supports basic auth
- iCalendar parsing with `ical` crate; recurrence rules via `rrule`
- Events can have recurrence info; editing a recurring event creates an override instance
- All-day events handled differently from timed events (no timezone)

### Encryption
- `ENCRYPTION_KEY` env var is hashed with HKDF-SHA256 to derive a 32-byte key
- Credentials encrypted with AES-256-GCM; stored as base64 in database
- Each encrypted field includes a random nonce (not reused)

### Frontend-Backend Communication
- Frontend is static HTML/CSS/JS built by Astro
- HTMX (`hx-get`, `hx-post`, `hx-swap`) fetches fragments from `/api/` endpoints
- Backend endpoints return HTML snippets (via Askama templates) or JSON
- User info fetched on page load via `hx-get="/api/user"`

## Notes

- **No CI/CD configured**: No GitHub Actions, linters, formatters, or test runners
- **No root workspace**: `frontend/` and `backend/` are independent packages; no root `package.json` or Cargo workspace
- **Session storage**: In-memory only; sessions lost on restart. For production, consider persistent session store
- **Reverse proxy required**: App does not handle TLS; recommended to run behind Caddy, nginx, or Traefik
- **Podman compatible**: Works as drop-in replacement for Docker
