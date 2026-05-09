# Caley

CalDAV calendar viewer with OIDC authentication.

## Architecture

- **backend/** — Rust (edition 2024) Axum server. Fetches events from a CalDAV server, renders HTML via Askama templates. All endpoints require OIDC authentication.
- **frontend/** — Astro 6 static site. Pico CSS + HTMX + Sass. Built to `dist/` and served by the backend as a fallback.

## Local Development

### Backend

Requires `.env` or environment variables with:

| Variable | Description |
|---|---|
| `OIDC_ISSUER` | OIDC provider issuer URL |
| `OIDC_CLIENT_ID` | OIDC client ID |
| `OIDC_CLIENT_SECRET` | OIDC client secret |
| `APP_BASE_URL` | Public URL of this app (e.g. `https://calendar.mydomain.ltd`) |
| `OIDC_LOGOUT` | Set to `false` to skip OIDC logout (default: `true`) |
| `OIDC_USER_ID_CLAIM` | Which OIDC claim to use as user ID: `sub` (default), `preferred_username`, `email`, or `name` |
| `CALDAV_PORT` | Listen port (default: `3000`) |
| `DATABASE_PATH` | Path to SQLite database file (default: `./data/caley.db`). Parent directory is created automatically. |
| `STATIC_DIR` | Path to frontend dist directory (default: `../frontend/dist`) |

CalDAV credentials (base URL, username, password) are configured per-user through the settings page after login, not via environment variables.

### Frontend

```bash
cd frontend
npm install
npm run dev     # dev server on localhost:4321
npm run build   # static build to dist/
```

## Docker

### Build

```bash
docker build -t caley .
```

### Run

```bash
docker run -p 3000:3000 \
  -v caley-data:/data \
  -e OIDC_ISSUER=https://auth.mydomain.ltd \
  -e OIDC_CLIENT_ID=caley \
  -e OIDC_CLIENT_SECRET=secret \
  -e APP_BASE_URL=http://localhost:3000 \
  caley
```

All environment variables from the `.env` section above can be passed with `-e`. `STATIC_DIR` is preset to `/static` in the image and doesn't need to be overridden. Mount a volume at `/data` (or your configured `DATABASE_PATH`) to persist the SQLite database across container restarts.