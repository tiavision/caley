# AGENTS.md

## Structure

Two-package monorepo with no root-level tooling:

- **`backend/`** — Rust binary crate (edition 2024). CalDAV client that fetches and parses iCal events. Axum web server.
- **`frontend/`** — Astro 6 static site. Pico CSS + HTMX + Sass. Calendar UI wired to the backend.

## Backend

- Run: `cargo run` from `backend/` → `localhost:3000`
- Requires `.env` in `backend/` with:
  - `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` — OIDC provider configuration
  - `OIDC_LOGOUT` — set to `false` to skip OIDC provider logout and just clear the session (default: `true`)
  - `APP_BASE_URL` — public URL of this app (e.g. `http://localhost:3000`), used for OIDC callback
  - `CALDAV_PORT` — listen port (default: `3000`)
  - `DATABASE_PATH` — path to SQLite database file (default: `./data/caley.db`)
  - `OIDC_USER_ID_CLAIM` — which OIDC claim to use as user ID: `sub` (default), `preferred_username`, `email`, or `name`
- CalDAV credentials (base URL, username, password) are per-user settings stored in SQLite, not global env vars
- New users are redirected to `/settings` to configure their CalDAV connection; settings are tested before saving
- Uses `dotenvy` to load `.env`; `clap` with derive + env feature for CLI args
- Auth: all endpoints require OIDC authentication via `axum-oidc`; unauthenticated requests redirect to the OIDC provider
- Session store: in-memory (`MemoryStore` from `tower-sessions`); sessions lost on restart
- Database: SQLite via `rusqlite` (bundled); per-user CalDAV settings stored in `user_settings` table
- Database file location is configurable via `DATABASE_PATH` env var (default: `./data/caley.db`). The parent directory is created automatically. When running in Podman/Docker, mount a volume at `/data` (or your configured path) to persist the database across container restarts, e.g. `podman run -v caley-data:/data …`
- Rust edition 2024 (not 2021)  
- No tests configured yet

## Frontend

- Install: `npm install` from `frontend/`
- Dev server: `npm run dev` → `localhost:4321`
- Build: `npm run build` → `frontend/dist/`
- TypeScript strict mode (extends `astro/tsconfigs/strict`)
- SCSS via `sass-embedded` (not dart-sass)
- HTMX is a dependency but not yet integrated into components

## Notes

- No CI, no linter, no formatter, no test runner configured in either package
- No root `package.json` or workspace config — each package is independent
- Frontend data is all hardcoded; backend is not yet serving an API
- OIDC authentication protects all endpoints; username shown in top-right via HTMX
