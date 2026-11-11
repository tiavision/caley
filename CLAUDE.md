# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Caley** is a personal groupware web app. It is no longer only a CalDAV calendar viewer: the app now includes calendar management, CardDAV contacts, IMAP/SMTP email, full-text search, email identities, image privacy controls, and WebAuthn-backed OpenPGP key management.

The repository is a two-package monorepo with independent frontend and backend tooling:

- **`backend/`** — Rust 2024 Axum binary. Handles auth, CalDAV/CardDAV/IMAP/SMTP clients, SQLite persistence, encryption, sync tasks, SSE, and static frontend serving.
- **`frontend/`** — Vite static multi-page app: HTML pages compiled by sinter (`@tiaforge-ui/sinter`), temper CSS (`@tiaforge-ui/temper`, `data-*` attributes) + Sass for app-specific rules, sinter's built-in messages (inlang message files) for i18n.

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
- `embed.rs`: the pages' API data in the HTML (`sinter-axum`). The frontend build (`embed: true`) leaves an empty `<script type="application/json" data-sn="…">` per resource whose request the page URL decides, listed in `dist/sinter-embed.json`; the layer streams the head at once, calls the API routes in-process with the page request's cookie and writes each response in. It sits inside `RequireAuthLayer` (signed-out app pages redirect first). Resolvers: the login page's `user` (401 → `null`) and search's `results` (its `src` calls `urlSearch()`, rebuilt in Rust). An API error or a 500 ms timeout ends the page with an error text (`<body data-sn-failed>`). The pages are read at startup: **restart the backend after `npm run build`**. Without `sinter-embed.json` the pages are served as plain files.
- `etag.rs`: every 200 JSON GET gets a weak `ETag` (body hash) and `Cache-Control: private, no-cache`; a matching `If-None-Match` is a 304, and sinter skips parsing/re-rendering a reload whose `ETag` is unchanged.
- `static_files.rs`: serves the frontend build (`STATIC_DIR`) with cache headers: hashed files under `/assets/` (`<name>-<8 char hash>.<ext>`) `public, max-age=31536000, immutable`; HTML pages and unhashed files (favicon, manifest, `icons/`, logo) `no-cache` (revalidated with ETag/Last-Modified), so a deploy reaches the next page load. Pages with embedded data are answered by `embed.rs` instead (`private, no-cache`, no validator). The whole app is compressed (`CompressionLayer` in `main.rs`: br/zstd/gzip/deflate; tower-http's default skips `text/event-stream`, images and bodies under 32 bytes).

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

Vite builds one HTML entry per page, each a sinter page (markup in the HTML file, entry script in `src/main/<page>.ts`):

- `/` — login page (`index.html`).
- `/calendar/` — calendar app.
- `/contacts/` — contacts app.
- `/mail/` — mail app.
- `/mail/view/` — standalone mail message view (no nav rail).
- `/search/` — calendar search page.
- `/settings/` — settings and encryption management.

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

- TypeScript strict mode. Pages are plain HTML compiled by sinter (`@tiaforge-ui/sinter`); the reference is `node_modules/@tiaforge-ui/sinter/llms.txt`. Data lives in `<sn-resource>`s, live updates come from `<sn-sync src="/api/events/stream">` + `<sn-event>`, mutations use `data-sn-post/put/patch/delete` with `data-sn-then`/`catch`/`optimistic`, UI state uses `data-sn-state`/`let`/`model`, lists and conditionals use `<template data-sn-for/if>`, dialogs are native `<dialog>`s, shared markup is a component (`src/components/<name>.html`), the app pages share one layout (`src/layouts/app-layout.html`). Prefer plain HTML/CSS, then sinter directives, then helpers, then actions; component scripts only for layout measuring, foreign DOM (iframes) or timers.
- `vite.config.ts`: `sinter({ typecheck: true, embed: true, lazy: ['openpgp'] })` compiles every HTML entry and runs `sinter check` on `vite build` (type errors fail it). `package.json` `sideEffects` lists the modules that act on import (styles, `src/styles/temper.ts`, `src/islands/*.ts`, page entries, `src/sinter.config.ts`; component scripts are not among them): a new one of those must be added there, or the build drops it.
- First paint (sinter's, see its `llms.txt` "First paint" and "Messages"). Each built page has two classic scripts first in its head: `assets/sinter-head-<hash>.js` (shared: the early runtime and the language picker: stored `locale`, else `navigator.languages`, else `en`; sets `<html lang>` and starts the page's language file `assets/<page>-<locale>-<hash>.js` without waiting for it, the English text is in the HTML and hidden for German until rendered) and the page's `assets/<page>-head-<hash>.js`. Nothing holds the paint: a `MutationObserver` renders each binding outside templates that needs no data (nav labels, `aria-current`, persisted state such as `nav-collapsed`) as the parser inserts its element, and the bindings over embedded data (`embed.rs`), `data-sn-for` rows included. The page bundle is `async` and takes the rendered DOM over once the document is parsed. Embedded resources (literal or `$url`-only `src`: `/api/user`, `/api/calendars`, `/api/email/accounts`, …) arrive in the HTML; the early file requests those whose URL reads early state, persisted values, `$now` and helpers (the calendar's week events and month grid, the mail list); the runtime takes those responses, so keep such URLs free of data from other resources if they should load early. So keep page chrome such as the nav rail outside `data-sn-if`/`data-sn-for` templates (inside them it waits for the page bundle), keep helpers that early bindings call light (they are copied into the early file; the build prints each early file's size), and remember that page-bundle code (`src/islands/`, `src/sinter.config.ts`) may run before the body is parsed: no DOM access at module top level.
- Navigation (sinter, automatic): every page carries an inline `<script type="speculationrules">` (prefetch of the app's other pages' HTML; allowed by `'inline-speculation-rules'` in the CSP's `script-src` in `main.rs`), and a link about to be followed (hover 65 ms, focus, press) requests the target page's literal-URL resources ahead (`sinter({ prefetch: false })` / `<html data-sn-prefetch="off">` / `<sn-resource early="off">` opt out). Keep GET endpoints free of side effects. temper turns on the cross-document view transitions (`@view-transition`, not with reduced motion); `app.scss` only names and animates the rail and `<main>`.
- `src/sinter.config.ts` (imported by every page): 401 → remember the URL and go to `/`, `sessionProbe: '/api/user'`, retry policies (`@tiaforge-ui/sinter/retry`: every GET is tried again after a network error, `network`; the mail flag PATCHes opt in with `data-sn-retry="flags"`; only idempotent writes may opt in, the backend ignores `Idempotency-Key`), allowed URL schemes, the page-wide listeners of `src/islands/common.ts`. sinter is the locale source (no `configure({ locale })`).
- `src/sinter.helpers.ts`: pure functions templates call by name. Messages are sinter's: `m.key()` in markup, `import { m } from '@tiaforge-ui/sinter/messages'` in TS (typed by the generated `src/sinter.messages.d.ts`; only `m.key`, never `m[key]`). Page tests import the helpers whole, so they and what they re-export stay light and pure: no openpgp/`lib/pgp/*`, no DOM side effects.
- `src/sinter.actions.ts`: one namespace per module in `src/actions/` (`mail`, `ics`, `pgpVault`, `pgpKeys`, `notify`, `share`, `maps`) for imperative work (crypto, files, MIME), called with `data-sn-call="ns.fn(…)"` or `<sn-resource call>`. Actions return data and never touch the DOM or sinter state. openpgp is a lazy package (`sinter({ lazy: ['openpgp'] })` in `vite.config.ts`, sinter's `docs/lazy.md`): `src/lib/pgp/*` and the actions import it as usual, sinter loads it on the first call. Every call of an openpgp export must be awaited or returned; `new`, `instanceof`, `openpgp.enums`/`config` and helpers reaching it are SN324, and openpgp in a page bundle through another package is SN325. The mail pages load it once they have taken over: `data-sn-import="openpgp"` on `<mail-reader>`'s root; settings and calendar on first use. A failed load is a `SinterError` (`stale` when a deploy removed the chunk): a catch on a call that may reach openpgp wraps its own text in the helper `loadErrorOr($error, …)` (`src/lib/errors.ts`: `m.pgp_loadError()` for a stale page), and `lib/pgp` and the actions let a `SinterError` through instead of turning it into their own error (a failed load is not a message that failed to decrypt or a bad signature).
- `src/sinter.types.ts`: the types `type="…"` may name; re-export each by name (no `export type *`).
- `src/components/`: sinter components (docs: `node_modules/@tiaforge-ui/sinter/docs/components.md`): `app-head` (a head component), `app-nav`, `nav-toggle`, event detail/form, thread, import, share menu, mail reader/compose/…; a `<name>.ts` beside one is its script (`month-fit`, `event-reminders`, `mail-frame`, `mail-split`, `mail-header-fit`, `mail-hidden-watch`, `mail-notifier`, `close-after`, `auth-return`), which writes only the component's state, emits its declared events and owns only foreign DOM (the mail iframe). Types come from the generated `src/sinter.components.d.ts`. `src/layouts/app-layout.html`: the document of the pages with the rail. `src/islands/`: page-wide listeners, no elements (`common.ts`: `data-shortcut` keys, the drawer on `pageswap`; `login.ts`, `settings.ts`). `src/lib/`: shared TS (API, formatting, mail utils and types, PGP, i18n). `src/styles/`: `temper.ts` plus per-page Sass.
- Page wiring: an app page is `<app-layout data-sn-prop="{ here: '/page/', title: 'Caley - Page' }" data-sn-state="{ … }">` (`title` is the whole tab title, `Caley` without one) → the page's resources and `<sn-sync>` → `<main>`, then `<script type="module" src="/src/main/<page>.ts">` (styles). The layout's tag takes DOM-event handlers only (they go onto `<body>`, page-wide keys); handle custom and component events on an element around their source, and put `data-sn-let` / `data-sn-persist` on an element inside. The layout declares `user` (read `user.map_url`, don't redeclare it); `<app-nav>` declares its own `<sn-sync>` for new-mail notifications; `<sn-sync>`s with the same URL share one connection, so pages declare their own only for their own events. `<nav-toggle>` opens the rail as a drawer on narrow screens. The login page and `/mail/view/` have their own document with `<app-head>`. Don't name state `page` (shadows `page()`).
- Conventions: components that declare a resource are used at page level, never inside `data-sn-if`/`data-sn-for`. A component sees only its props, its own state and the page's resources; it reports through declared events (`data-sn-emits`), never by writing page state. A dialog that edits a draft (`<event-form>`, `<import-dialog>`, `<mail-compose>`, `<mail-origins>`, `<mail-account>`, `<mail-folder>`) wraps the part of the page that opens it (its root has `display: contents`): an opener emits the draft (`emit('event-new', newEventDraft(…))`), the component's root handler sets its own `draft` and calls `showModal`, so the form is rendered before the dialog opens. Changes to mail messages reach both lists (`messages`, `results`) through the server's events (`email_updated`, `email_removed`), not through patches of another resource: `patch(item)` / `remove(item)` act on the resource the item came from. An optimistic change stays when the request succeeds and rolls back on error, so `data-sn-then` only writes what the server returned beyond it (`upsert(…, $result)`, `refresh(…)`), never the same change again. Custom events (`emit()`, islands' `CustomEvent`s, `dismiss`) reach every ancestor that handles them, inner first, until a handler calls `$event.preventDefault()`; `page()` does nothing past the first or last page, so it needs no `$meta.x.hasNext &&` guard. Persisted UI state uses `data-sn-persist="<key>"` (localStorage `sn:<key>`: `mail`, `calendar-mode` (the calendar's view; the old `calendar-view` held an object and is no longer read), `nav-collapsed`, `event-default-calendar`, `caldav_base_url`); keep keys stable.
- Tests sit next to the markup (`<page>/index.test.ts`, `src/components/*.test.ts`) and use `@tiaforge-ui/sinter/testing`: `render(html, { ...app, requests: mockRequests({...}), actions, helpers, streams: { '/api/events/stream': mockStream() }, storage: memoryStorage() })`, then `flush()`/`settle()`. `app` (`src/test-components.ts`) passes the components and the layout (`render(…, { components, layouts })`); `render()` compiles as the build does (constants written in, decided branches); component scripts are left out (jsdom has no layout or ResizeObserver) unless a test passes them (`scripts: { 'close-after': scripts['close-after'] }`, or a stub class). Import HTML with `?raw`; `src/test-dom.ts` adds what jsdom lacks (modal dialogs, `hidePopover`, `commandfor` invokers).
- Styling uses temper (`@tiaforge-ui/temper`): semantic HTML plus `data-*` variants, light/dark automatic. The reference is `node_modules/@tiaforge-ui/temper/llms-full.txt`. App-specific CSS goes in `src/styles/*.scss` with `--tm-*` tokens, no hard-coded colors or outer margins.
- Icons are `data-icon="<lucide name>"`; the temper Vite plugin emits only names that appear as static strings in the markup.
- The temper Vite plugin runs `temper check` on every build (errors fail it). Check rendered pages with `npm run inspect` (backend in demo mode on :3000): it builds, logs in and runs temper-inspect on every page of the build the backend serves from `dist`; add `-- --sheet <dir>` for screenshots or `-- /mail/` for one page.
- temper-inspect's known gaps (findings to read, not to fix in Caley): it finds a dialog's opener only through `commandfor`/`popovertarget`, and in the built page sinter's `showModal('id')` handlers are compiled away. Caley opens dialogs with `showModal()` (the sinter variant, for readability), so those dialogs are opened directly, without the state a click would set: "opened directly … nearly empty" warnings are expected, and the contents of dialogs that are not empty go unchecked silently. Its "covered" check tests an element's box centre, so a wrapped inline link (the event location) is reported as covered by its own `<dd>`; on narrow screens with a message open, the reader overlay covers the mail list on purpose. `temper check` warns TM109 for `.reader`'s `data-grow` because it does not see through `<mail-reader>`'s `display: contents` root. Pass URLs with state to reach more dialogs (`npm run inspect -- "/search/?q=a" "/mail/?account_id=demo-work&mailbox=INBOX&uid=3"`).
- i18n messages live in `frontend/messages/{locale}.json` (`project.inlang/settings.json`: `en` base, `de`), plain strings with `{param}`s; a missing `de` key falls back to `en` and is reported at build. The language is switched with `$locale = 'de'` (settings), without a reload.
- `vite.config.ts` proxies `/api`, `/auth`, and `/logout` to `http://localhost:3000`; the dev server is usually `localhost:5173`. After a dependency change restart it with `npx vite --force`.
- Agents verify against the build, never the Vite dev server (its hot reloads navigate pages mid-check): `npm run build`, then (re)start the DEMO_MODE backend on `http://localhost:3000`, which serves `dist` with embedded data (it reads the pages at startup; WebAuthn works there too, since that origin is `APP_BASE_URL`). Agents working in parallel build into their own `--outDir` and serve it with `npx vite preview --outDir <dir> --port <port>`, which proxies `/api` like the dev server (no embedded data there: the browser requests it).

## Demo Mode

Set `DEMO_MODE=true` to start the backend in a fully self-contained demo mode — no external network calls are made. Use this to test the full UI end-to-end without any CalDAV, CardDAV, IMAP, or SMTP infrastructure.

- Uses an isolated `./data/demo.db` database (never touches the configured `DATABASE_PATH`).
- Accepts any credentials on the login page — always binds to the fixed demo user.
- Seeds realistic fixture data on startup (3 calendars, ~15 events, 8 contacts, 10 inbox messages). Uses `INSERT OR IGNORE` so data seeded in a previous run is preserved.
- CalDAV/CardDAV write ops (create/edit/delete event, contact) update the local DB only.
- SMTP sends are logged to stdout and silently discarded.
- Email autodiscovery returns a fixture response without HTTP calls.
- OIDC is not available in demo mode. WebAuthn/PGP work (passkey with PRF, vault, keys) when the page origin is `APP_BASE_URL` or listed in `WEBAUTHN_EXTRA_ORIGINS` (e.g. `http://localhost:5173` for Vite).

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
npm run inspect   # build, then temper-inspect on every page, logged in (needs a DEMO_MODE backend on :3000)
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
- Static file caching relies on hashed asset names: never serve unhashed files from `/assets/`, and keep HTML `no-cache`. A tab left open across a deploy can still request an old hashed chunk (openpgp, loaded lazily) that the new build no longer has: sinter reports it as `$error.stale` (nothing old is kept on the server).
- The app does not terminate TLS. Use a reverse proxy in production and keep `APP_BASE_URL` set to the public HTTPS origin, especially for OIDC and WebAuthn.
