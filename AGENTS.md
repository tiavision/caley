# AGENTS.md

## Structure

Two-package monorepo with no root-level tooling:

- **`backend/`** — Rust binary crate (edition 2024). CalDAV client that fetches and parses iCal events. Axum web server.
- **`frontend/`** — Astro 6 static site. Pico CSS + HTMX + Sass. Calendar UI wired to the backend.

## Backend

- Run: `cargo run` from `backend/` → `localhost:3000`
- Configuration is via environment variables. A `.env` file in `backend/` is also loaded automatically.
- Required: `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`, `APP_BASE_URL`, `ENCRYPTION_KEY`
- Optional: `CALDAV_PORT` (default: `3000`), `OIDC_LOGOUT` (default: `true`), `OIDC_USER_ID_CLAIM` (default: `sub`), `DATABASE_PATH` (default: `./data/caley.db`), `STATIC_DIR` (optional, defaults to `../frontend/dist`)
- CalDAV credentials (base URL, username, password) are per-user settings stored encrypted in SQLite with AES-256-GCM
- New users are redirected to `/settings` to configure their CalDAV connection; settings are tested before saving
- Uses `dotenvy` to load `.env`; `clap` with derive + env feature for CLI args
- Auth: all endpoints require OIDC authentication via `axum-oidc`; unauthenticated requests redirect to the OIDC provider
- Session store: in-memory (`MemoryStore` from `tower-sessions`); sessions lost on restart
- Database: SQLite via `rusqlite` (bundled); per-user CalDAV settings stored encrypted (AES-256-GCM) in `user_settings` table; encryption key derived from `ENCRYPTION_KEY` env var via HKDF-SHA256
- Database file location is configurable via `DATABASE_PATH` env var (default: `./data/caley.db`). The parent directory is created automatically. When running in Podman/Docker, mount a volume at `/data` (or your configured path) to persist the database across container restarts, e.g. `podman run -v caley-data:/data …`
- Rust edition 2024
- No tests configured yet

## Frontend

- Install: `npm install` from `frontend/`
- Dev server: `npm run dev` → `localhost:4321`
- Build: `npm run build` → `frontend/dist/`
- TypeScript strict mode (extends `astro/tsconfigs/strict`)
- SCSS via `sass-embedded` (not dart-sass)
- HTMX integrated into components and backend templates for dynamic loading, calendar navigation, and user info

## Notes

- No CI, no linter, no formatter, no test runner configured in either package
- No root `package.json` or workspace config — each package is independent
- OIDC authentication protects all endpoints; username shown in top-right via HTMX
