# AGENTS.md

## Structure

Two-package monorepo with no root-level tooling:

- **`backend/`** — Rust binary crate (edition 2024). CalDAV client that fetches and parses iCal events. Axum web server.
- **`frontend/`** — Astro 6 static site. Pico CSS + HTMX + Sass. Calendar UI wired to the backend.

## Backend

- Run: `cargo run` from `backend/` → `localhost:3000`
- Requires `.env` in `backend/` with `BASE_URL`, `USERNAME`, `PASSWORD` (clap reads these via `#[arg(env)]`)
- Uses `dotenvy` to load `.env`; `clap` with derive + env feature for CLI args
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
