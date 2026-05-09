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
| `CALDAV_BASE_URL` | CalDAV server URL |
| `CALDAV_USERNAME` | CalDAV username |
| `CALDAV_PASSWORD` | CalDAV password |
| `OIDC_ISSUER` | OIDC provider issuer URL |
| `OIDC_CLIENT_ID` | OIDC client ID |
| `OIDC_CLIENT_SECRET` | OIDC client secret |
| `APP_BASE_URL` | Public URL of this app (e.g. `https://calendar.mydomain.ltd`) |
| `OIDC_LOGOUT` | Set to `false` to skip OIDC logout (default: `true`) |
| `CALDAV_PORT` | Listen port (default: `3000`) |
| `STATIC_DIR` | Path to frontend dist directory (default: `../frontend/dist`) |

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
  -e CALDAV_BASE_URL=https://caldav.mydomain.ltd \
  -e CALDAV_USERNAME=user \
  -e CALDAV_PASSWORD=pass \
  -e OIDC_ISSUER=https://auth.mydomain.ltd \
  -e OIDC_CLIENT_ID=caley \
  -e OIDC_CLIENT_SECRET=secret \
  -e APP_BASE_URL=http://localhost:3000 \
  caley
```

All environment variables from the `.env` section above can be passed with `-e`. `STATIC_DIR` is preset to `/static` in the image and doesn't need to be overridden.