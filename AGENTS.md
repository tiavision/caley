# AGENTS.md

## Structure

Two-package monorepo with no root-level tooling:

- **`backend/`** — Rust binary crate (edition 2024). CalDAV client that fetches and parses iCal events. Axum web server.
- **`frontend/`** — Vite + Svelte 5 static site. Pico CSS + Sass. Calendar UI wired to the backend.

## Backend

- Run: `cargo run` from `backend/` → `localhost:3000`
- Configuration is via environment variables. A `.env` file in `backend/` is also loaded automatically.
- Always required: `APP_BASE_URL`, `ENCRYPTION_KEY`
- OIDC mode (all three must be set): `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`
- Optional: `CALDAV_PORT` (default: `3000`), `OIDC_LOGOUT` (default: `true`), `OIDC_USER_ID_CLAIM` (default: `sub`), `DATABASE_PATH` (default: `./data/caley.db`), `STATIC_DIR` (defaults to `../frontend/dist`), `SYNC_INTERVAL_MINS` (default: `5`)
- CalDAV credentials (base URL, username, password) are per-user settings stored encrypted in SQLite with AES-256-GCM
- Uses `dotenvy` to load `.env`; `clap` with derive + env feature for CLI args
- Session store: in-memory (`MemoryStore` from `tower-sessions`); sessions lost on restart
- Database: SQLite via `sqlx` (bundled); per-user CalDAV settings stored encrypted (AES-256-GCM) in `user_settings` table; encryption key derived from `ENCRYPTION_KEY` env var via HKDF-SHA256
- Database file location is configurable via `DATABASE_PATH` env var (default: `./data/caley.db`). The parent directory is created automatically.
- Rust edition 2024
- No tests configured yet

## Authentication modes

The app supports two auth modes selected at startup based on env vars:

**OIDC mode** (when `OIDC_ISSUER`, `OIDC_CLIENT_ID`, and `OIDC_CLIENT_SECRET` are all set):
- Uses `axum-oidc`; unauthenticated requests redirect to the OIDC provider
- User ID extracted from the configured OIDC claim (`OIDC_USER_ID_CLAIM`)
- CalDAV credentials configured separately in the settings page after OIDC login

**Local CalDAV login mode** (when any OIDC var is absent):
- No external identity provider needed
- Users log in at `/settings/` with CalDAV server URL, username, and password
- User ID derived as `username@hostname` (e.g. `user@dav.myurl.tld`)
- Display name fetched from the CalDAV server via PROPFIND on the principal URL
- Credentials validated and saved in one step; session stored server-side
- `POST /auth/login` handles login; `GET /logout` clears the session

Both modes use a `CurrentUser` extractor (`oidc.rs`) that reads from OIDC claims or the session depending on mode. All protected handlers use `CurrentUser` — not `OidcClaims` directly.

## Account deletion

`DELETE /api/account` (both modes): deletes all user data from the database. In local mode also flushes the session and the frontend redirects to `/settings/`.

## Frontend

- Install: `npm install` from `frontend/`
- Dev server: `npm run dev` → `localhost:5173` (proxies `/api`, `/auth`, `/logout` to backend)
- Build: `npm run build` → `frontend/dist/`
- TypeScript strict mode
- SCSS via `sass-embedded` (not dart-sass)
- i18n via `@inlang/paraglide-js` (compile-time message generation)
- The settings page (`/settings/`) doubles as the login page in local mode: it calls `/api/user` on mount and if it gets a 401 switches into login mode (different heading, submits to `/auth/login` instead of `/api/settings`)

## Notes

- No CI, no linter, no formatter, no test runner configured in either package
- No root `package.json` or workspace config — each package is independent
