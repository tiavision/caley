# AGENTS.md

## Structure

Two-package monorepo with no root-level tooling:

- **`backend/`** — Rust 2024 Axum binary. Handles auth, CalDAV/CardDAV/IMAP/SMTP clients, SQLite persistence, encryption, background sync, SSE, WebAuthn/OpenPGP endpoints, and static frontend serving.
- **`frontend/`** — Vite static multi-page app of sinter-compiled HTML pages (`@tiaforge-ui/sinter`). temper CSS (`@tiaforge-ui/temper`, `data-*` attributes) + Sass for app-specific rules, TypeScript strict mode, i18n via `@inlang/paraglide-js`.

There is no root `package.json`, Cargo workspace, or shared command runner. Work inside the package directory.

## App Capabilities

Caley is a personal groupware app, not just a calendar viewer:

- Calendar: CalDAV accounts, calendar listing/filtering, event create/edit/delete, recurring event overrides, ICS import/preview, calendar search.
- Contacts: CardDAV addressbooks, contact list/search/filtering, create/edit/delete contacts, contact photos, address map links.
- Email: IMAP accounts, autodiscovery, SMTP sending, multiple identities, mailboxes, unread counts, threads, flags/read state, move/delete, attachments, export, virtual folders, full-text mail search, remote image privacy controls, new-mail SSE notifications.
- Encryption: WebAuthn/passkey enrollment, OpenPGP public key storage, synced encrypted private-key vault, device approval flow, key import, own-key identity management, trusted sender keys, PGP-aware mail display and compose paths.

## Backend

- Run: `cargo run` from `backend/` -> `localhost:3000`.
- Check/test: `cargo check`, `cargo test` from `backend/`.
- Configuration is via CLI/env vars; `backend/.env` is loaded automatically by `dotenvy`.
- Required: `APP_BASE_URL`, `ENCRYPTION_KEY`.
- OIDC is additive when all three are set: `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`.
- Optional: `CALDAV_PORT` (default `3000`), `OIDC_LOGOUT` (default `true`), `OIDC_USER_ID_CLAIM` (default `sub`), `DATABASE_PATH` (default `./data/caley.db`), `STATIC_DIR` (default `../frontend/dist`), `SYNC_INTERVAL_MINS` (default `5`), `SEARCH_MODE` (`default`, `trigram`, or Snowball algorithm name such as `German`), `MAP_URL` (URL template with `{q}` placeholder), `WEBAUTHN_EXTRA_ORIGINS`.
- SQLite via `sqlx` with bundled SQLite; migrations live in `backend/migrations/`.
- Database parent directory is created automatically.
- Sessions are stored in SQLite via `tower-sessions-sqlx-store` and expire after 7 days of inactivity.
- CalDAV and IMAP passwords are encrypted at rest with AES-256-GCM. The key is derived from `ENCRYPTION_KEY` via HKDF-SHA256.
- PGP private keys are managed through the frontend vault flow and stored only as encrypted vault/key records.

## Authentication

CalDAV-credential login is always available; OIDC login is shown alongside it when configured.

- Unified login page: `/`.
- `POST /auth/login`: validates CalDAV credentials and starts a session.
- `/auth/oidc`: OIDC login trigger/callback. Register `{APP_BASE_URL}/auth/oidc` at the provider.
- `GET /logout`: unified logout.
- `GET /api/auth/config`: public endpoint reporting OIDC availability.
- Protected handlers should use `CurrentUser` from `oidc.rs`, which reads OIDC claims first and then the server-side session.
- Do not use `OidcClaims` directly in route handlers.
- `RequireAuthLayer` allows public routes/static assets and otherwise requires OIDC claims or a session user.

## Important Backend Modules

- `main.rs`: config, state, routes, auth layers, WebAuthn setup, security headers.
- `oidc.rs`, `local_auth.rs`: auth/session/current-user behavior.
- `settings.rs`, `caldav.rs`, `calendar.rs`, `sync.rs`: CalDAV settings, calendar/event APIs, background calendar sync.
- `carddav.rs`, `contacts.rs`: CardDAV addressbook/contact support.
- `email.rs`, `email_db.rs`, `email_sync.rs`, `imap_client.rs`, `email_autodiscover.rs`: email accounts, sync/cache, SMTP, identities, mail APIs.
- `pgp.rs`: WebAuthn and OpenPGP vault/public-key endpoints.
- `crypto.rs`: encrypted-at-rest helpers.
- `search.rs`: calendar/mail FTS normalization and query generation.
- `sse.rs`: live calendar/contact/email update events.

## Frontend

- Install: `npm install` from `frontend/`.
- Dev server: `npm run dev` -> usually `localhost:5173`, proxying `/api`, `/auth`, and `/logout` to the backend.
- Build: `npm run build` -> `frontend/dist/`.
- Preview: `npm run preview`.
- Tests: `npm test` (Vitest with jsdom).
- SCSS uses `sass-embedded`, not dart-sass.
- Pages: `/`, `/calendar/`, `/contacts/`, `/mail/`, `/mail/view/`, `/search/`, `/settings/`.
- Each page is an HTML file compiled by sinter (`<page>/index.html`, entry script `src/main/<page>.ts`); shared markup lives in `partials/`, imperative code in `src/actions/`, custom elements in `src/islands/`. See CLAUDE.md "Frontend Notes" for the conventions.
- `settings/` includes CalDAV/account settings and encryption/key management (`partials/pgp-settings.html`); `mail/` is the main mail UI with PGP-aware viewing/compose (`partials/mail-reader.html`, `partials/mail-compose.html`, `src/actions/mail.ts`).

## Account Deletion

`DELETE /api/account` deletes all user data from the database. In local login mode it also clears the session and redirects to `/`.

## Demo Mode

Set `DEMO_MODE=true` to start the backend without any external infrastructure. Use this to test the full UI end-to-end.

```bash
cd backend
DEMO_MODE=true APP_BASE_URL=http://localhost:3000 ENCRYPTION_KEY=demo-key cargo run
```

- Uses an isolated `./data/demo.db` (never touches the configured `DATABASE_PATH`).
- Accepts any credentials on the login page — always binds to the fixed demo user `demo@demo.example`.
- Seed data is inserted with `INSERT OR IGNORE` on startup, so the DB survives restarts and the user can make edits that persist. Account rows (IMAP/CalDAV credentials) use `ON CONFLICT DO UPDATE` so re-keying with a different `ENCRYPTION_KEY` is corrected automatically.
- Pre-seeded: 3 calendars, ~16 events (including recurring), 1 addressbook, 31 contacts, 2 IMAP accounts (Work + Personal), 6 mailboxes each, 22+ messages with styled HTML bodies.
- All outbound network calls are suppressed: CalDAV PUT/DELETE, IMAP connect (sync, IDLE, flag updates, delete, move), SMTP send, email autodiscovery.
- SMTP compose returns 200 OK without sending. Flag changes, deletes, and moves update only the local DB.
- OIDC is not available in demo mode. WebAuthn/PGP work when the page origin is `APP_BASE_URL` or listed in `WEBAUTHN_EXTRA_ORIGINS`.

## Notes

- No CI is configured.
- Keep the independent package structure unless explicitly asked to change it.
- Preserve existing encrypted-at-rest patterns when adding stored credentials or secrets.
- OpenPGP private-key material should remain client-side decrypted only.
- The app does not terminate TLS; production deployments should use a reverse proxy and set `APP_BASE_URL` to the public HTTPS origin.
