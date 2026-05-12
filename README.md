# Caley

CalDAV calendar viewer with OIDC authentication.

## Architecture

- **backend/** — Rust (edition 2024) Axum server. Fetches events from a CalDAV server, renders HTML via Askama templates. All endpoints require OIDC authentication.
- **frontend/** — Astro 6 static site. Pico CSS + HTMX + Sass. Built to `dist/` and served by the backend as a fallback.

## Local Development

### Backend

All configuration is via environment variables. A `.env` file in `backend/` is also loaded automatically if present.

| Variable | Description | Required? |
|---|---|---|
| `OIDC_ISSUER` | OIDC provider issuer URL | Yes |
| `OIDC_CLIENT_ID` | OIDC client ID | Yes |
| `OIDC_CLIENT_SECRET` | OIDC client secret | Yes |
| `APP_BASE_URL` | Public URL of this app (e.g. `https://calendar.mydomain.ltd`) | Yes |
| `ENCRYPTION_KEY` | Passphrase used to derive an AES-256-GCM key for encrypting stored CalDAV credentials | Yes |
| `CALDAV_PORT` | Listen port (default: `3000`) | No |
| `OIDC_LOGOUT` | Set to `false` to skip OIDC logout (default: `true`) | No |
| `OIDC_USER_ID_CLAIM` | OIDC claim to use as user ID: `sub` (default), `preferred-username`, `email`, or `name` | No |
| `DATABASE_PATH` | Path to SQLite database file (default: `./data/caley.db`) | No |
| `STATIC_DIR` | Path to frontend dist directory (default: `../frontend/dist`) | No |

CalDAV credentials (base URL, username, password) are configured per-user through the settings page after login, not via environment variables. Credentials are encrypted at rest using a key derived from `ENCRYPTION_KEY`.

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
  -e ENCRYPTION_KEY=your-passphrase \
  caley
```

All environment variables from the table above can be passed with `-e`. `STATIC_DIR` is preset to `/static` in the image and doesn't need to be overridden. Mount a volume at `/data` (or your configured `DATABASE_PATH`) to persist the SQLite database across container restarts.

### Docker Compose

```yaml
services:
  caley:
    image: caley
    ports:
      - "3000:3000"
    volumes:
      - caley-data:/data
    environment:
      - OIDC_ISSUER=https://auth.example.com
      - OIDC_CLIENT_ID=caley
      - OIDC_CLIENT_SECRET=secret
      - APP_BASE_URL=https://calendar.example.com
      - ENCRYPTION_KEY=your-passphrase
volumes:
  caley-data:
```

### Podman

Podman works as a drop-in replacement. Use `podman build` and `podman run` with the same flags as above, or `podman-compose` with the compose file.

## OIDC Provider

The app requires an OIDC provider for authentication. [Pocket ID](https://pocket-id.org/) has been tested and confirmed working. Other providers implementing the OIDC standard (Keycloak, Authelia, Authentik, etc.) should work as well.

## Reverse Proxy

This app does not handle TLS termination. Running behind a reverse proxy (Caddy, nginx, Traefik) is recommended for HTTPS. When behind a proxy, set `APP_BASE_URL` to the public `https://` URL of the application.