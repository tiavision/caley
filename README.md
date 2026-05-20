# Caley

CalDAV calendar viewer. Supports two authentication modes:

- **OIDC** — authenticate via an external OIDC provider (Pocket ID, Keycloak, Authelia, Authentik, …). Configure CalDAV credentials separately in the settings page after login.
- **Local (CalDAV login)** — no OIDC required. Users log in directly with their CalDAV server URL, username, and password. The user ID is derived as `username@hostname` (e.g. `user@dav.myurl.tld`). The display name is fetched from the CalDAV server.

## Architecture

- **backend/** — Rust (edition 2024) Axum server. CalDAV client, iCalendar parser, SQLite cache. Serves the frontend as static files.
- **frontend/** — Vite + Svelte 5 static site. Pico CSS + Sass. Calendar UI wired to the backend via fetch/SSE.

## Local Development

### Backend

All configuration is via environment variables. A `.env` file in `backend/` is also loaded automatically if present.

| Variable | Description | Required? |
|---|---|---|
| `APP_BASE_URL` | Public URL of this app (e.g. `https://calendar.mydomain.ltd`) | Yes |
| `ENCRYPTION_KEY` | Passphrase used to derive an AES-256-GCM key for encrypting stored CalDAV credentials | Yes |
| `OIDC_ISSUER` | OIDC provider issuer URL | OIDC mode only |
| `OIDC_CLIENT_ID` | OIDC client ID | OIDC mode only |
| `OIDC_CLIENT_SECRET` | OIDC client secret | OIDC mode only |
| `CALDAV_PORT` | Listen port (default: `3000`) | No |
| `OIDC_LOGOUT` | Set to `false` to skip OIDC logout (default: `true`) | No |
| `OIDC_USER_ID_CLAIM` | OIDC claim to use as user ID: `sub` (default), `preferred-username`, `email`, or `name` | No |
| `DATABASE_PATH` | Path to SQLite database file (default: `./data/caley.db`) | No |
| `STATIC_DIR` | Path to frontend dist directory (default: `../frontend/dist`) | No |
| `SYNC_INTERVAL_MINS` | Background sync interval in minutes (default: `5`) | No |
| `SEARCH_MODE` | Full-text search strategy: `default` (unicode-aware prefix matching), `trigram` (substring matching, any language), or a Snowball algorithm name for language-specific stemming: `Arabic`, `Danish`, `Dutch`, `English`, `Finnish`, `French`, `German`, `Greek`, `Hungarian`, `Italian`, `Norwegian`, `Portuguese`, `Romanian`, `Russian`, `Spanish`, `Swedish`, `Tamil`, `Turkish`. Changing this rebuilds the search index on next start. (default: `default`) | No |

If all three OIDC variables are set, OIDC mode is used. Otherwise the app starts in local CalDAV login mode — no OIDC provider needed.

CalDAV credentials (base URL, username, password) are encrypted at rest using a key derived from `ENCRYPTION_KEY`.

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
# OIDC mode
docker run -p 3000:3000 \
  -v caley-data:/data \
  -e OIDC_ISSUER=https://auth.mydomain.ltd \
  -e OIDC_CLIENT_ID=caley \
  -e OIDC_CLIENT_SECRET=secret \
  -e APP_BASE_URL=http://localhost:3000 \
  -e ENCRYPTION_KEY=your-passphrase \
  caley

# Local CalDAV login mode (no OIDC)
docker run -p 3000:3000 \
  -v caley-data:/data \
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
      - OIDC_ISSUER=https://auth.example.com   # omit all three OIDC_ vars for local login mode
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

When using OIDC mode, [Pocket ID](https://pocket-id.org/) has been tested and confirmed working. Other providers implementing the OIDC standard (Keycloak, Authelia, Authentik, etc.) should work as well.

If no OIDC provider is available, omit the three `OIDC_*` variables entirely to use local CalDAV login instead.

## Account Management

Users can delete their account (all stored credentials and cached data) from the settings page. In local login mode this also ends the session and redirects back to the login form.

## Reverse Proxy

This app does not handle TLS termination. Running behind a reverse proxy (Caddy, nginx, Traefik) is recommended for HTTPS. When behind a proxy, set `APP_BASE_URL` to the public `https://` URL of the application.
