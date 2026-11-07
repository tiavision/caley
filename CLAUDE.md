# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Caley** is a personal groupware web app. It is no longer only a CalDAV calendar viewer: the app now includes calendar management, CardDAV contacts, IMAP/SMTP email, full-text search, email identities, image privacy controls, and WebAuthn-backed OpenPGP key management.

The repository is a two-package monorepo with independent frontend and backend tooling:

- **`backend/`** — Rust 2024 Axum binary. Handles auth, CalDAV/CardDAV/IMAP/SMTP clients, SQLite persistence, encryption, sync tasks, SSE, and static frontend serving.
- **`frontend/`** — Vite + Svelte 5 static multi-page app. temper CSS (`@tiaforge-ui/temper`, `data-*` attributes) + Sass for app-specific rules, compile-time i18n via `@inlang/paraglide-js`.

There is no root-level package manager or Cargo workspace.

## Authentication

CalDAV-credential login is **always** available. OIDC login is **additive** and shown alongside it when configured. The unified login page lives at `/`.

**CalDAV-credential login**:

- `POST /auth/login` validates CalDAV credentials with PROPFIND, stores them encrypted, and creates a session.
- User ID is derived as `username@hostname`.
- Display name is fetched from the CalDAV principal when possible.

**OIDC login**:

- Enabled only when `OIDC_ISSUER`, `OIDC_CLIENT_ID`, and `OIDC_CLIENT_SECRET` are all set.
- Login goes through `/auth/oidc`; the IdP redirect URI must be `{APP_BASE_URL}/auth/oidc`.
- CalDAV, email, and encryption settings are configured inside the app after OIDC login.
- User ID comes from `OIDC_USER_ID_CLAIM` (`sub` by default).

`CurrentUser` in `oidc.rs` is the unified extractor for protected handlers. It tries OIDC claims first, then the server-side session. Do not use `OidcClaims` directly in handlers.

`RequireAuthLayer` allows public paths (`/`, `/auth/*`, `/logout`, `/api/auth/config`, `/api/push/webhook`, `/api/email/navigate`, and static assets) and requires either OIDC claims or a session user for everything else. Unauthenticated API requests return 401; page requests redirect to `/`.

Sessions are stored in SQLite via `tower-sessions-sqlx-store` and expire after 7 days of inactivity.

## Backend

### Key Modules

- `main.rs`: CLI/env config, database init, state construction, WebAuthn setup, routing, auth layers, security headers.
- `oidc.rs`: `CurrentUser`, auth config, user info, logout, auth gating.
- `local_auth.rs`: CalDAV login, user ID derivation, account deletion.
- `settings.rs`: CalDAV account settings, multiple CalDAV account CRUD, connection validation.
- `caldav.rs`, `calendar.rs`, `sync.rs`: CalDAV client, iCalendar parsing, calendar/event CRUD, imports, recurring event handling, background sync.
- `carddav.rs`, `contacts.rs`: CardDAV addressbook/contact sync and contact CRUD.
- `email.rs`, `email_db.rs`, `email_sync.rs`, `imap_client.rs`, `email_autodiscover.rs`: IMAP account setup, autodiscovery, mailbox/message cache, SMTP sending, virtual folders, identities, search, attachments, image privacy controls, background/IDLE sync.
- `pgp.rs`: WebAuthn credential enrollment/authentication, synced OpenPGP vault endpoints, public key storage, raw message access for PGP processing.
- `crypto.rs`: HKDF-SHA256 key derivation and AES-256-GCM encryption for stored secrets.
- `search.rs`: `SEARCH_MODE` parsing and FTS query/index text normalization.
- `sse.rs`: Server-sent events for calendar, contacts, and email updates.

### Configuration

`dotenvy` loads `backend/.env` automatically. Required variables:

- `APP_BASE_URL`
- `ENCRYPTION_KEY`

Optional variables:

- `CALDAV_PORT` — listen port, default `3000`.
- `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` — all three enable OIDC.
- `OIDC_LOGOUT` — default `true`.
- `OIDC_USER_ID_CLAIM` — default `sub`.
- `DATABASE_PATH` — default `./data/caley.db`; parent directory is created automatically.
- `STATIC_DIR` — default `../frontend/dist`.
- `SYNC_INTERVAL_MINS` — calendar/email sync interval, default `5`.
- `SEARCH_MODE` — `default`, `trigram`, or a Snowball algorithm name such as `German` or `English`.
- `MAP_URL` — map fallback URL template with `{q}` placeholder.
- `WEBAUTHN_EXTRA_ORIGINS` — additional allowed WebAuthn origins.

### Storage

SQLite is the only database. Migrations live in `backend/migrations/`.

Important persisted data:

- Users and encrypted CalDAV accounts.
- Cached calendars, events, recurrence-derived search data, and hidden calendars.
- Cached CardDAV addressbooks and contacts, including contact photos/addresses.
- Encrypted IMAP accounts, mailboxes, messages, addresses, attachments, identities, virtual folders, image allow/block lists, unread state, and local `.eml` files under the database data directory.
- WebAuthn credentials.
- OpenPGP public keys.
- Synced encrypted OpenPGP private-key vault data, devices, and vault wraps.
- SQLite-backed session data.

Secrets such as CalDAV and IMAP passwords are encrypted at rest using AES-256-GCM with a key derived from `ENCRYPTION_KEY` via HKDF-SHA256. PGP private keys are handled by the frontend vault flow and stored as encrypted key records.

### Main API Surface

Auth/account:

- `POST /auth/login`
- `GET /auth/oidc`
- `GET /logout`
- `GET /api/auth/config`
- `GET /api/user`
- `DELETE /api/account`

Calendar/search:

- `GET /api/calendars`
- `POST /api/calendar/toggle`
- `GET|POST /api/calendar`
- `GET /api/events`
- `POST|DELETE /api/event`
- `POST /api/import`
- `POST /api/import/preview`
- `GET /api/search`
- `GET /api/events/stream`
- `POST /api/push/webhook`

CalDAV/CardDAV settings and contacts:

- `GET /api/settings/form`
- `POST /api/settings`
- `GET /api/caldav/accounts`
- `POST|PUT|DELETE /api/caldav/account`
- `GET /api/contacts`
- `POST|DELETE /api/contact`
- `GET /api/contact/photo`

Email:

- `GET /api/email/autodiscover`
- `GET /api/email/accounts`
- `POST|PUT|DELETE /api/email/account`
- `GET /api/email/mailboxes`
- `POST /api/email/sync`
- `GET /api/email/unread-counts`
- `GET /api/email/folders`
- `POST|PUT|DELETE /api/email/folder`
- `GET /api/email/messages`
- `GET /api/email/message`
- `GET /api/email/thread`
- `PATCH /api/email/message/flags`
- `POST /api/email/message/move`
- `DELETE /api/email/message`
- `POST /api/email/send`
- `POST /api/email/send-raw`
- `GET /api/email/search`
- `GET /api/email/suggest-addrs`
- `GET /api/email/proxy-image`
- `GET /api/email/attachment`
- `GET /api/email/export`
- `GET|POST|DELETE /api/email/image-origins`
- `GET|POST|DELETE /api/email/image-blacklist`
- `GET /api/email/identities`
- `POST|PUT|DELETE /api/email/identity`
- `GET /api/email/navigate` public helper route.

WebAuthn/OpenPGP:

- `POST /api/webauthn/register/start`
- `POST /api/webauthn/register/finish`
- `POST /api/webauthn/authenticate/start`
- `POST /api/webauthn/authenticate/finish`
- `GET /api/webauthn/credentials`
- `DELETE /api/webauthn/credential`
- `GET|PUT|DELETE /api/pgp/vault`
- `POST /api/pgp/vault/initialize`
- `POST /api/pgp/devices/pending`
- `GET /api/pgp/keys`
- `POST|DELETE /api/pgp/key`
- `GET /api/email/raw`

## Frontend

### Pages

Vite builds multiple HTML entries:

- `/` — login page (`LoginPage.svelte`).
- `/calendar/` — calendar app (`CalendarApp.svelte`).
- `/contacts/` — contacts app (`ContactsPage.svelte`).
- `/mail/` — mail app (`MailApp.svelte`).
- `/mail/view/` — standalone mail message view.
- `/search/` — calendar search page (`SearchPage.svelte`).
- `/settings/` — settings and encryption management (`SettingsForm.svelte`).

### User-Visible Capabilities

Calendar:

- Month/week-style calendar UI, active-calendar filtering, event create/edit/delete, recurring event override support, ICS import/preview, location map fallback.

Contacts:

- CardDAV addressbook listing, contact search/filtering, create/edit/delete contacts, emails/phones/addresses/org fields, contact photos, native `geo:` map launch with URL fallback.

Email:

- IMAP account setup with autodiscovery and SMTP settings.
- Multiple email accounts and identities.
- Mailbox browsing, unread counts, message pagination, search filters, threads, flag/read state, move/delete, attachments, export, and new-mail notifications.
- SMTP compose/send, raw MIME send, and protected MIME generation for PGP flows.
- Remote image privacy controls through image origin allow/block lists and same-origin image proxying.
- PGP-aware message display: encrypted/signed detection, decryption, signature verification, public key attachment handling, Autocrypt key discovery.

Encryption/settings:

- WebAuthn/passkey enrollment using PRF-capable credentials.
- Synced encrypted OpenPGP private-key vault.
- Device approval/pending-device flow.
- Private key import, own public key management, identity management on keys.
- Trusted sender public key import/delete.
- Unlock/lock session state for local PGP operations.

### Frontend Notes

- Svelte 5 with TypeScript strict mode.
- Styling uses temper (`@tiaforge-ui/temper` 0.1.0): semantic HTML plus `data-*` variants, light/dark automatic. The reference is `node_modules/@tiaforge-ui/temper/llms-full.txt`. Entry styles: `src/styles/temper.ts` (temper modules, `virtual:temper-icons.css`, `app.scss`). Scoped CSS only for app-specific parts, with `--tm-*` tokens, no hard-coded colors or outer margins.
- Icons are `data-icon="<lucide name>"`; the temper Vite plugin emits only names that appear as static strings in the markup.
- The Vite plugin runs `temper check` on every build (errors fail it). Check rendered pages with `npm run inspect` (dev server running, backend in demo mode): it logs in and runs temper-inspect on every page; add `-- --sheet <dir>` for screenshots or `-- /mail/` for one page.
- Pages use `<body data-shell="fixed">`; `initRail()` (`src/main/nav.ts`) mounts `NavRail.svelte` as a direct child of body, and `NavToggle.svelte` opens it as a drawer on narrow screens.
- i18n messages live in `frontend/messages/`.
- `vite.config.ts` proxies `/api`, `/auth`, and `/logout` to `http://localhost:3000`.
- The dev server defaults to Vite's port, usually `localhost:5173`.
- There is a Vitest test script for frontend tests.

## Demo Mode

Set `DEMO_MODE=true` to start the backend in a fully self-contained demo mode — no external network calls are made. Use this to test the full UI end-to-end without any CalDAV, CardDAV, IMAP, or SMTP infrastructure.

- Uses an isolated `./data/demo.db` database (never touches the configured `DATABASE_PATH`).
- Accepts any credentials on the login page — always binds to the fixed demo user.
- Seeds realistic fixture data on startup (3 calendars, ~15 events, 8 contacts, 10 inbox messages). Uses `INSERT OR IGNORE` so data seeded in a previous run is preserved.
- CalDAV/CardDAV write ops (create/edit/delete event, contact) update the local DB only.
- SMTP sends are logged to stdout and silently discarded.
- Email autodiscovery returns a fixture response without HTTP calls.
- OIDC and WebAuthn/PGP are not available in demo mode.

```bash
# Run backend in demo mode (frontend dev server still needed separately)
cd backend
DEMO_MODE=true APP_BASE_URL=http://localhost:3000 ENCRYPTION_KEY=demo-key cargo run
```

## Development Commands

Backend:

```bash
cd backend
cargo run
cargo check
cargo build --release
cargo test
```

Frontend:

```bash
cd frontend
npm install
npm run dev
npm run build
npm run preview
npm test
npm run inspect   # temper-inspect on every page, logged in (needs the dev server and a DEMO_MODE backend)
```

No root-level commands are configured.

## Docker

Build:

```bash
docker build -t caley .
```

Run with local CalDAV login only:

```bash
docker run -p 3000:3000 \
  -v caley-data:/data \
  -e APP_BASE_URL=https://calendar.example.com \
  -e ENCRYPTION_KEY=your-passphrase \
  caley
```

Run with OIDC also enabled:

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

The image builds the frontend with Node, builds the Rust backend, and serves the static frontend from the backend. Persist `/data` or the configured database path so SQLite data, cached mail files, sessions, credentials, and vault metadata survive restarts.

## Implementation Notes

- Keep handlers using `CurrentUser`; do not split auth behavior by handler unless a route is intentionally public.
- Preserve the independent package structure. Do not add root tooling unless explicitly requested.
- Respect existing encrypted-at-rest patterns for new stored credentials or secrets.
- Email sync and calendar/contact sync both use SSE to refresh live UI state.
- OpenPGP private-key material should remain client-side decrypted only. Backend PGP endpoints store encrypted vault records and public metadata.
- Security headers in `main.rs` intentionally restrict script/image/frame behavior; update CSP carefully when adding frontend capabilities.
- The app does not terminate TLS. Use a reverse proxy in production and keep `APP_BASE_URL` set to the public HTTPS origin, especially for OIDC and WebAuthn.
