# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Caley** is a personal groupware web app. It is no longer only a CalDAV calendar viewer: the app now includes calendar management, CardDAV contacts, IMAP/SMTP and JMAP email, full-text search, email identities, image privacy controls, and WebAuthn-backed OpenPGP key management.

The repository is a monorepo of independent packages (frontend, backend, and the app shell), each with its own tooling:

- **`backend/`** — Rust 2024 Axum binary. Handles auth, CalDAV/CardDAV/IMAP/SMTP/JMAP clients, SQLite persistence, encryption, sync tasks, SSE, and static frontend serving.
- **`frontend/`** — Vite static multi-page app: HTML pages compiled by sinter (`@tiaforge-ui/sinter`), temper CSS (`@tiaforge-ui/temper`, `data-*` attributes) + Sass for app-specific rules, sinter's messages (the inlang project, read by the content module `src/sinter.content.ts`) for i18n.
- **`native/`** — Tauri 2 app shell (Android first, also desktop; iOS possible): a server picker that opens a Caley server's own pages in the app's webview. See Native App.

There is no root-level package manager or Cargo workspace.

## Authentication

CalDAV-credential login is **always** available. OIDC login is **additive** and shown alongside it when configured. The unified login page lives at `/` (`/en/`, `/de/`: the pages are built per language, see Frontend → Pages). Signed in, `/` is a redirect into the app before any HTML is sent (`landing.rs`), so the login page never flashes.

**CalDAV-credential login**:

- `POST /auth/login` validates CalDAV credentials with PROPFIND, stores them encrypted, and creates a session.
- User ID is derived as `username@hostname`.
- Display name is fetched from the CalDAV principal when possible.

**OIDC login**:

- Enabled only when `OIDC_ISSUER`, `OIDC_CLIENT_ID`, and `OIDC_CLIENT_SECRET` are all set.
- Login goes through `/auth/oidc`; the redirect URI (and the post-logout URL) is built from the request's origin (`public_origin.rs`: forwarding headers from a `TRUSTED_PROXIES` peer, else `http://` + `Host`, else `APP_BASE_URL`) and kept in the session for the token request, so the app works both behind a proxy and accessed directly; each origin's `{origin}/auth/oidc` must be registered at the IdP.
- CalDAV, email, and encryption settings are configured inside the app after OIDC login.
- User ID comes from `OIDC_USER_ID_CLAIM` (`sub` by default).
- The callback keeps the sign-in's access and refresh tokens, encrypted (`oidc_tokens.rs`, table `oidc_tokens`), for JMAP accounts that sign in with the user's own OIDC login (`auth = 'oidc'`): the sync sends the access token as its bearer token and renews it at the provider's token endpoint when it expires (no refresh token, or a refused one: the user signs in again). `OIDC_SCOPES` adds scopes to the sign-in: `offline_access` for the refresh token, and what the mail server wants (Stalwart 1.x as the identity provider: `urn:ietf:params:oauth:scope:mail`, without which its session lists no mail account; an external IdP that Stalwart trusts: its audience/scope). `/api/user`'s `oidc_signed_in` says whether tokens are kept (the account dialog offers the OIDC sign-in then).

`CurrentUser` in `oidc.rs` is the unified extractor for protected handlers. It tries OIDC claims first, then the server-side session. Do not use `OidcClaims` directly in handlers.

`RequireAuthLayer` allows public paths (the login page `/` or `/<lang>/`, `/auth/*`, `/logout`, `/api/auth/config`, `/api/push/webhook`, `/api/email/navigate`, and static assets) and requires either OIDC claims or a session user for everything else. Unauthenticated API requests return 401; page requests redirect to the login page in their language with the page (`/de/?next=/de/mail/?uid=3`; with OIDC, to `/auth/oidc?auto=1`, the page kept in the session).

Where a sign-in lands (`landing.rs`): the `next` page (kept in the session from `/?next=…` or the redirect above, used once), else the app page the user loaded last (`last_page` in the session, its path with the language prefix, no query; prefetches and `/mail/view/` not counted), else `/calendar/` (in the login page's language). Signed-in `GET /` (or `/de/`) redirects there; `POST /auth/login` answers it as `{"redirect": …}`, which the login form's `data-sn-then` goes to with `navigate()`; the OIDC callback redirects there. The session is lost at logout or expiry, and `last_page` with it.

Sessions are stored in SQLite via `tower-sessions-sqlx-store` and expire after 7 days of inactivity.

## Backend

### Key Modules

- `api.rs`: the JSON API under `/api`, described by OpenAPI (utoipa, utoipa-axum). Each module's `routes()` registers its handlers from their `#[utoipa::path]` annotations (`routes!(…)`, paths relative to `/api`), so the router and the spec cannot disagree; `api::private()`/`api::public()` are nested under `/api` in `main.rs`. Responses are `ToSchema` structs (no `json!` bodies), query parameters `IntoParams`, forms and multipart bodies `request_body(… content_type …)`; the described structs are the handlers' extractors (no hand parsing): `axum_extra::extract::Query`/`Form` (serde_html_form: repeated keys into a `Vec`; an optional value sent empty is `None` with `#[serde(default, deserialize_with = "crate::api::empty_as_none")]`), multipart `axum_typed_multipart::TypedMultipart` (a JSON field is `api::JsonField<T>`), errors `ApiError` (`{"error"}`) or plain text with a description, binary bodies `api::Binary`. An `Option` field serde always writes (no `skip_serializing_if`) is `#[schema(required)]` (present, maybe `null`). The spec is `frontend/openapi.json`, committed for the dev server, `npm test` and `sinter check`: the test `openapi_spec_is_current` fails while it is stale, `UPDATE_OPENAPI=1 cargo test openapi` writes it. The Docker image does not rely on the committed copy: it builds the backend first, writes the binary's own spec with `caley --openapi` (prints it and exits, no configuration needed) and builds the frontend against that. A new or changed route: annotate it, regenerate, then `npx sinter check --strict` in frontend/ shows what the pages must change.
- `main.rs`: CLI/env config, database init, state construction, WebAuthn setup, routing, auth layers, security headers.
- `public_origin.rs`: the origin a request was made to (`RequestOrigin` extractor), for OIDC redirect/post-logout URLs.
- `oidc.rs`: `CurrentUser`, auth config, user info, logout, auth gating.
- `landing.rs`: where `/` and a sign-in send someone signed in (`next`, `last_page`, the calendar); language prefixes of paths.
- `local_auth.rs`: CalDAV login, user ID derivation, account deletion.
- `settings.rs`: CalDAV account settings, multiple CalDAV account CRUD, connection validation.
- `caldav.rs`, `calendar.rs`, `sync.rs`: CalDAV client, iCalendar parsing, calendar/event CRUD, imports, recurring event handling, background sync.
- `carddav.rs`, `contacts.rs`: CardDAV addressbook/contact sync and contact CRUD.
- `email.rs`, `email_db.rs`, `email_sync.rs`, `imap_client.rs`, `email_autodiscover.rs`: IMAP account setup, autodiscovery, mailbox/message cache, SMTP sending, virtual folders, identities, search, attachments, image privacy controls, background/IDLE sync.
- `jmap/`: JMAP accounts (RFC 8620/8621), beside IMAP in the same `imap_accounts` table (`protocol`, `jmap_url`, `auth`: `password` Basic, `bearer` an API token, `oidc` the user's OIDC tokens (only when the mail server trusts Caley's own identity provider), `oauth` a sign-in at the mail server itself (`oauth.rs`)). `mod.rs` the client (session from `/.well-known/jmap`, its URLs resolved against the session URL, method calls, blob download/upload, EventSource push); `sync.rs` fills the same cache an IMAP sync fills (rows, `.eml` files, server events), so the API and the pages do not tell them apart: each mailbox an email is in gives it a UID there (`jmap_messages`, UIDs from `jmap_mailboxes.next_uid`, never reused; a move is a removal and a new message, as in IMAP), mailbox names are paths with `/`, the inbox `INBOX`; the first sync lists the account (Email/query), later ones Email/changes since the kept state (`jmap_states`; `cannotCalculateChanges`/`invalidArguments` lists it again), a renamed mailbox is listed again under its new name; sources are downloaded by `blobId` and parsed by `imap_client`'s parsers (`header_from_bytes` from the email's raw `headers`, `parse_body_bytes`); push replaces IDLE. `ops.rs`: flags/moves/deletions as Email/set (a delete unlinks an email in another mailbox too), sending as upload + Email/import into the `sent`-role mailbox + EmailSubmission/set with the server's identity for the address. Emptying the trash by date answers `savedate_unsupported` (no SAVEDATE in JMAP). Saving a JMAP account fetches its session first; autodiscovery also tries `/.well-known/jmap` on the domain.
- `jmap/oauth.rs`: a JMAP account that signs in at the mail server (`auth = 'oauth'`), apart from Caley's own login: the authorization code flow with PKCE against the server's OAuth endpoints (RFC 8414 metadata on the JMAP server's origin), with the OAuth client the server has for Caley (its id and secret entered in the account dialog, kept with the account, the secret encrypted; none for a public client). Saving such an account (new, on another server or client, without tokens, or `authorize`) saves nothing yet: the account waits in the session (`Pending`, under its `state`) and the answer's `authorize` link (`/auth/jmap?start=…`, the page `navigate()`s there) redirects to the server's sign-in; the server sends the browser back to `/auth/jmap` (the redirect URI to register at the server: `{origin}/auth/jmap`), the code is exchanged, the JMAP session fetched with the token, and only then the account saved with its tokens (`jmap_oauth_tokens`, encrypted) and the browser sent to `/mail/?account_id=…`, or to `/mail/?account_error=…` (shown above the mail). Scopes: `offline_access urn:ietf:params:oauth:scope:mail`; the access token is renewed with the refresh token (`oauth::access_token`), a refused renewal is `SignInAgain` (edit the account, "Sign in at the server again").
- `oidc_tokens.rs`: the users' OIDC tokens (above), renewed with the refresh token; `token_request` is the token endpoint call both use.
- `pgp.rs`: WebAuthn credential enrollment/authentication, synced OpenPGP vault endpoints, public key storage, raw message access for PGP processing.
- `crypto.rs`: HKDF-SHA256 key derivation and AES-256-GCM encryption for stored secrets.
- `search.rs`: `SEARCH_MODE` parsing and FTS query/index text normalization.
- `sse.rs`: Server-sent events for calendar, contacts, and email updates.
- `notifications.rs`: Web Push to the user's devices while no page is open (the app's UnifiedPush registrations, `native/`): subscriptions per device (`notification_subscriptions`), the payload encrypted for the device (RFC 8291 `aes128gcm`) and signed with the server's VAPID key (RFC 8292; made once, kept encrypted in `server_secrets`). The SSE broadcaster hands it every `NewEmail` (`SseBroadcaster::set_notifier`), whether a page is connected or not; an endpoint answering 404/410 is dropped. A subscription may give the device's timezone (IANA).
- `reminders.rs`: calendar reminders by Web Push. A task once a minute sends, for each user with a device, the alarms due in their active calendars (raw events read and expanded as `GET /api/events` does, 15 days ahead), each once (`sent_reminders`, kept 3 days; an alarm missed by up to 10 minutes, e.g. a restart, still goes out). Alarms of all-day and floating events are timed in each device's timezone and sent to the devices in it; the notification's tag is `reminder|<uid>|<fire epoch seconds>`, the one the calendar page gives its own (`event-reminders.ts`), so a page's and a pushed reminder (or two tabs') replace each other. Not `/api/push/…`: that is the CalDAV servers' push to Caley (`sync.rs`).
- `embed.rs`: the pages' API data in the HTML (`sinter-axum`). The frontend build (`embed: true`) leaves an empty `<script type="application/json" data-sn="…">` per resource whose request the page URL decides, listed in `dist/sinter-embed.json`; the layer streams the head at once, calls the API routes in-process with the page request's cookie and writes each response in. It sits inside `RequireAuthLayer` (signed-out app pages redirect first). Resolvers, registered for each language's file (`de/index.html:user`; the languages from the manifest's `locales`): only the login page's `user` (401 → `null`); every other placeholder is filled from what the build describes (a `src` of literals and `$url.<name>` reads: the search page's `results` from `?q=&start=&end=&cal=`). Mail's `thread` and `threadEvents` are `.skip`ped: their message comes through `urlRef()`, which the build cannot describe, so the browser loads them. A placeholder the build cannot describe that is neither resolved nor skipped stops the server at startup. Prefer a `src` that reads `$url.<name>` directly over a resolver that parses the URL again in Rust. The layer also answers an unprefixed page URL with a 302 to the visitor's language (`locale` cookie, which every page sets, else `Accept-Language`, else `en`; `Vary: Cookie, Accept-Language`). An API error or a 500 ms timeout ends the page with an error text (`<body data-sn-failed>`). The pages are read at startup: **restart the backend after `npm run build`**. Without `sinter-embed.json` the pages are served as plain files.
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
- `OIDC_SCOPES` — scopes asked for besides `openid profile`, space-separated (`offline_access urn:ietf:params:oauth:scope:mail` for Stalwart as the identity provider; see Authentication).
- `DATABASE_PATH` — default `./data/caley.db`; parent directory is created automatically.
- `STATIC_DIR` — default `../frontend/dist`.
- `SYNC_INTERVAL_MINS` — calendar/email sync interval, default `5`.
- `SEARCH_MODE` — `default`, `trigram`, or a Snowball algorithm name such as `German` or `English`.
- `MAP_URL` — map fallback URL template with `{q}` placeholder.
- `WEBAUTHN_EXTRA_ORIGINS` — additional allowed WebAuthn origins.
- `TRUSTED_PROXIES` — IPs/CIDRs (comma-separated) whose `Forwarded` / `X-Forwarded-*` headers give a request's origin (`public_origin.rs`).

### Storage

SQLite is the only database. Migrations live in `backend/migrations/`.

Important persisted data:

- Users and encrypted CalDAV accounts.
- Cached calendars, events, recurrence-derived search data, and hidden calendars.
- Cached CardDAV addressbooks and contacts, including contact photos/addresses.
- Encrypted IMAP and JMAP accounts (a JMAP account's mailbox ids, email ids and sync states besides), mailboxes, messages, addresses, attachments, identities, virtual folders, image allow/block lists, unread state, and local `.eml` files under the database data directory.
- WebAuthn credentials.
- OpenPGP public keys.
- Synced encrypted OpenPGP private-key vault data, devices, and vault wraps.
- SQLite-backed session data.
- The users' OIDC access and refresh tokens (encrypted).

Secrets such as CalDAV and IMAP passwords are encrypted at rest using AES-256-GCM with a key derived from `ENCRYPTION_KEY` via HKDF-SHA256. PGP private keys are handled by the frontend vault flow and stored as encrypted key records.

### Main API Surface

The full, typed description is `frontend/openapi.json` (generated, see `api.rs`); the list below is an overview.

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
- `POST /api/import` (into a calendar, or a new one by name)
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
- `POST|DELETE /api/email/folder` (POST creates or, with `folder_id`, updates)
- `GET /api/email/messages`
- `GET /api/email/message`
- `GET /api/email/thread`
- `PATCH /api/email/message/flags`
- `POST /api/email/message/move`
- `DELETE /api/email/message`
- `PATCH /api/email/messages/flags`, `POST /api/email/messages/move`, `POST /api/email/messages/delete` (bulk: the marked messages in one request, answering the ones that failed)
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

Notifications (Web Push to the user's devices):

- `GET /api/notifications/vapid`
- `PUT|DELETE /api/notifications/subscription`
- `POST /api/notifications/test`

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

Vite builds one HTML entry per page, each a sinter page (markup in the HTML file, entry script in `src/main/<page>.ts`). With `localePrefix` each is built once per language under its prefix (`/en/calendar/`, `/de/calendar/`: `dist/en/…`, `dist/de/…`), the language's messages and `<html lang>` written in; an unprefixed URL (below) redirects to the visitor's language:

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

- IMAP account setup with autodiscovery and SMTP settings; JMAP accounts (Stalwart, Fastmail, …) signed in with a password, an API token, a sign-in at the mail server (OAuth, with the server's client for Caley) or the user's OIDC sign-in.
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

- TypeScript strict mode. Pages are plain HTML compiled by sinter (`@tiaforge-ui/sinter`); the reference is `node_modules/@tiaforge-ui/sinter/llms.txt` (the short version; `llms-full.txt` is the whole one), and `npx sinter docs <topic | attribute | SNxxx>` prints one part of it, so look things up there instead of reading it again; `npx sinter render <page>.html --api http://localhost:3000` (or `--api mocks.json`, steps such as `--click '<selector>'`) runs a page on jsdom and prints its requests, markup and resources. Data lives in `<sn-resource>`s, live updates come from `<sn-sync src="/api/events/stream">` + `<sn-event>`, mutations use `data-sn-post/put/patch/delete` with `data-sn-then`/`catch`/`optimistic`, UI state uses `data-sn-state`/`let`/`model`, lists and conditionals use `<template data-sn-for/if>`, dialogs are native `<dialog>`s, shared markup is a component (`src/components/<name>.html`), the app pages share one layout (`src/layouts/app-layout.html`). Prefer plain HTML/CSS, then sinter directives, then helpers, then actions; component scripts only for layout measuring, foreign DOM or timers.
- `vite.config.ts`: `sinter({ typecheck: true, strict: true, embed: true, localePrefix: true, swap: true, compileHints: true, lazy: ['openpgp', '@tiavision/sinter-mail'], libraries: ['@tiavision/sinter-mail'], … })` compiles every HTML entry and runs `sinter check` on `vite build`; strict mode (sinter's `docs/strict.md`) makes warnings fail like errors, in the build, `npx sinter check` and the dev server. Fix what a warning reports; keep a code a warning (`strict: { allow: { SNxxx: 'reason' } }`) only when the user agrees. `package.json` `sideEffects` lists the modules that act on import (styles, `src/styles/temper.ts`, `src/islands/*.ts`, page entries, `src/sinter.config.ts`; component scripts are not among them): a new one of those must be added there, or the build drops it. `src/sinter.content.ts` (sinter's content module) gives the site's languages (`languages()`, which `localePrefix` builds every page in) and the messages (`inlang()`), both from `project.inlang`.
- First paint (sinter's, see its `llms.txt` "First paint" and "Languages in the URL"). Each page is built per language (`localePrefix`), its text already in the HTML: no language file, nothing hidden until rendered. Each built page has two classic scripts first in its head: `assets/sinter-head-<hash>.js` (shared: the early runtime) and the page's `assets/<lang>-<page>-head-<hash>.js`. Nothing holds the paint: a `MutationObserver` renders each binding outside templates that needs no data (nav labels, `aria-current`, persisted state such as `nav-collapsed`) as the parser inserts its element, and the bindings over embedded data (`embed.rs`), `data-sn-for` rows included. The page bundle is `async` and takes the rendered DOM over once the document is parsed. Embedded resources (literal or `$url`-only `src`: `/api/user`, `/api/calendars`, `/api/email/accounts`, …) arrive in the HTML; the early file requests those whose URL reads early state, persisted values, `$now` and helpers (the calendar's week events and month grid, the mail list); the runtime takes those responses, so keep such URLs free of data from other resources if they should load early. So keep page chrome such as the nav rail outside `data-sn-if`/`data-sn-for` templates (inside them it waits for the page bundle), keep helpers that early bindings call light (they are copied into the early file; the build prints each early file's size), and remember that page-bundle code (`src/islands/`, `src/sinter.config.ts`) may run before the body is parsed: no DOM access at module top level.
- Navigation (sinter's swaps, `swap: true`, its `docs/navigation.md` "Swaps"): a link, `navigate()`, Back or Forward to another page of the app fetches that page's HTML (the same request a load sends, embedded data included) and swaps it in through the Navigation API, without a page load; a page in another language (the language pickers), a form's navigation, `/logout` and browsers without the Navigation API load the page. The rail is kept across swaps (`data-sn-permanent="rail"` on `<app-nav>`'s `<nav>`): the same element, its scroll and collapsed state; the next page's code takes over its bindings (`aria-current`), and its resources and `<sn-sync>` start again (the event stream reconnects at each swap: sinter's `docs/proposals/stream-handover.md` would keep it). Module code runs once per document: per-page work goes in a listener for `sn-page` (`document`, after each page mounts, the first included), never in an inline or classic script (SN540). The pages that swap carry no Speculation Rules (the swap fetches the page itself), so the CSP (`main.rs`) is `script-src 'self'`: turning swaps off would bring sinter's inline Speculation Rules back, which need `'inline-speculation-rules'` there. A link about to be followed (hover 65 ms, focus, press) requests the target page's literal-URL resources ahead (`sinter({ prefetch: false })` / `<html data-sn-prefetch="off">` / `<sn-resource early="off">` opt out). Keep GET endpoints free of side effects. Handlers go elsewhere with sinter's `navigate(url, { replace })` (after a sign-in, after the account is deleted), not with an island or component script; URLs built in code get the language prefix with the helper `pagePath()`. A link to the page itself that changes only query parameters the page reads stays on the page (sinter's same-page links), and a `$url` write from a user's event adds a history entry; `data-sn-model="$url.x"` binds a field to a URL parameter (the search page's dates). temper turns on the cross-document view transitions (`@view-transition`, not with reduced motion); `app.scss` only names and animates the rail and `<main>`.
- `src/sinter.config.ts` (imported by every page): 401 → the login page with this page as `next` (`loginUrl()`, `src/lib/return-url.ts`; the server keeps it, `landing.rs`), `sessionProbe: '/api/user'`, retry policies (`src/lib/retry-policies.ts`, also configured by the mail page tests; `@tiaforge-ui/sinter/retry`: every GET is tried again after a network error, `network`; the mail flag PATCHes opt in with `data-sn-retry="flags"`; only idempotent writes may opt in, the backend ignores `Idempotency-Key`), allowed URL schemes, the page-wide listeners of `src/islands/common.ts`. sinter is the locale source (no `configure({ locale })`).
- `src/sinter.helpers.ts`: pure functions templates call by name. Messages are sinter's: `m.key()` in markup, `import { m } from '@tiaforge-ui/sinter/messages'` in TS (typed by the generated `src/sinter.namespaces.d.ts`; only `m.key`, never `m[key]`; a TS file that imports `m` must not use `m` as another name, e.g. a lambda parameter, or the build reads `m.x` as a message). Page tests import the helpers whole, so they and what they re-export stay light and pure: no openpgp/`lib/pgp/*`, no DOM side effects.
- Reading a message (`mail.body()`, `src/actions/mail.ts`): every message is read in the browser from its source (`GET /api/email/message/raw`) by sinter-mail's `mail.mime` (its guide's "Whole messages", "Large attachments: cut messages" and "Encrypted and signed mail"); `/api/email/message` carries no body. A plain message is fetched cut (`?omit=65536`: the server leaves the bodies of large attachments out, `backend/src/email/cut.rs`, a port of sinter-mail's `mail.cut`, and says where they were in the `Mail-Holes` header; the whole message answers `Range` with its `ETag`), so its text renders without downloading the attachments. Every message is fetched cut first (an armored ciphertext is never cut, so it arrives whole); a cut the reader does not accept (`cut: 'invalid'`) or a cut ciphertext (a decrypt step `incomplete`) reads the whole message at once; a signature over a cut attachment (a verify step `incomplete`) leaves the body `pending`: shown, marked signed, with a "Verify signature" button (`mail.verify`, then `refresh(body)`) that reads the whole message and verifies it. Opening an encrypted part in a cut message (its step `<part>#cut`, or reason `'incomplete'`: its ciphertext was cut) reads the whole message and opens the part there by its id (`mail.openPart(msg, step, part.id)`); holes the reader refuses (a `TypeError`) read the whole message too. `mail.mime` lists the OpenPGP steps the app runs: `settle()` (`src/lib/pgp/steps.ts`, openpgp) decrypts with the unlocked vault keys and verifies with the stored trusted keys, each trusted only for the addresses it was stored under (`signer`), until no step is pending. caley has no MIME parser of its own. `src/lib/mail-reader.ts` (pure) turns the message into the reader's blocks (`ReaderBody.parts`, `textParts`: bodies in reading order, never joined; a forwarded message's after a line; an encrypted part from inside the message opened only when the reader asks, `mail.openPart`, and shown apart), the badges ("encrypted"/"verified" only for the whole message, a per-part mark otherwise, a warning when the sender is not the one proven), the protected subject and reply addresses, and `quotable` (a reply quotes decrypted text only when a good signature binds the sender). A decrypted part's inline images are data: URLs typed from their bytes (`mail.imageType`; the CSP's `img-src` has no `blob:`), any other part's the server serves by content id (`/api/email/proxy-image?…&cid=`); decrypted attachments are offered as files; attachments of plain mail are downloaded from the server (`/api/email/attachment`). Hiding the page (`mail.wipe()`) and locking the vault (`onLock`, `src/lib/pgp/session.ts`) call `mail.forget()`, which drops the decrypted plaintexts sinter-mail keeps. Helper modules must not import sinter-mail's functions (every page imports the helpers): the markup calls `email.parse`, `partOptions` gives the options.
- Mail bodies: sinter-mail (`@tiavision/sinter-mail`; its guide is `node_modules/@tiavision/sinter-mail/docs/mail.md`). The reader parses each HTML block in a let, `email.parse(part.html, options)` with `partOptions(…)` (`bodyImages` plus the part's `encrypted` and `legacyDisplay`, once `media` is known) (`email` is the package's helper namespace in `src/sinter.helpers.ts`; `bodyImages` in `src/lib/mail.ts`: remote images through `/api/email/proxy-image`, `cid:` images from the message's parts or a decrypted message's `cids`; `@media` rules for the reader's width, `media`: sinter-mail's `<mail-width>` beside the body reports `'narrow'` below 640px, `'wide'` above, and the mail is parsed once that is known, so a phone parses once; a decrypted body (`encrypted`) loads no remote image until the reader clicks "Load images" for that message (`consent`, reader state, never kept); its origins are still listed), and `<mail-body data-sn-with="{ body: tree }">` renders the tree as part of the page, built by sinter-mail's script builder (its `mail-view` script: elements created one by one, no HTML written): no iframe, no sanitizer (the parser keeps only what is safe, styles resolved and inline), `tree.images.origins` are the remote image origins. sinter-mail is a lazy package (`sinter({ lazy })`): its parser, its MIME reader and its component scripts (`<mail-body>`'s builder, `<mail-width>`, `<mail-warm>`) load on first use, off the mail pages' bundle, so a let that calls `email.parse` is `undefined` as a whole until then (not only the call: keep the parse in a let of its own, the reader's `trees`, so nothing else waits for it; `<email-reader>`'s root loads the package with `data-sn-import`), every `mail.*` call in the actions is awaited, and page tests get the namespace from `app.lazyHelpers` (`src/test-components.ts`). And `<email-reader>` (and compose's `<mail-body>`) carry `data-sn-early="off"`, keeping it and sinter-mail's components out of the early script; `<mail-warm>` (sinter-mail's) on both mail pages parses a sample mail while the page waits for the message, and `compileHints` (`vite.config.ts`) has V8 compile the page modules as they load, so the first real parse is not a cold one (`<mail-warm>` also starts loading the lazy package). sinter-mail's `mail.css` (imported by the mail pages' entries before `mail.scss`) gives back the browser's defaults where mails count on them (tables, images, content-box sizing, block spacing) and leaves the rest to temper; the reader's body is in its `.mail-scroll` box (a wide mail scrolls sideways). An HTML body takes the app's colours (dark mode included) unless it sets colours of its own without saying it works on a dark page (sinter-mail's `body.light`): then `<mail-body>` puts `.mail-light` on its root, a light canvas (`mail.css`; `mail.scss` adds temper's tokens mixed from it). A mail says so with its `color-scheme` (meta or property) or with `prefers-color-scheme: dark` rules (sinter-mail's `tree.schemes`); the colours such rules change are `light-dark()` in the tree and follow the page's scheme. Page tests get sinter-mail's components and its scripts (`mail-view`, without which `<mail-body>` renders nothing) from `app` (`src/test-components.ts`), and a `<mail-width>` that reports `wide` at once (jsdom has no layout; `mailWidth('narrow')` for a phone).
- `src/sinter.actions.ts`: one namespace per module in `src/actions/` (`mail`, `ics`, `pgpVault`, `pgpKeys`, `notify`, `share`, `maps`) for imperative work (crypto, files, MIME), called with `data-sn-call="ns.fn(…)"` or `<sn-resource call>`. Actions return data and never touch the DOM or sinter state. openpgp is a lazy package (`sinter({ lazy: ['openpgp'] })` in `vite.config.ts`, sinter's `docs/lazy.md`): `src/lib/pgp/*` and the actions import it as usual, sinter loads it on the first call. Every call of an openpgp export must be awaited or returned; `new`, `instanceof`, `openpgp.enums`/`config` and helpers reaching it are SN324, and openpgp in a page bundle through another package is SN325. The mail pages load it once they have taken over: `data-sn-import="openpgp"` on `<email-reader>`'s root; settings and calendar on first use. A failed load is a `SinterError` (`stale` when a deploy removed the chunk): a catch on a call that may reach openpgp wraps its own text in the helper `loadErrorOr($error, …)` (`src/lib/errors.ts`: `m.pgp_loadError()` for a stale page), and `lib/pgp` and the actions let a `SinterError` through instead of turning it into their own error (a failed load is not a message that failed to decrypt or a bad signature).
- APIs: `frontend/apis.ts` is `sinter({ apis })` for the build and the page tests (`app` in `src/test-components.ts` passes it with the parsed spec): the API `main` is `/api` with `openapi.json`, `defaults` say each list endpoint's `select`/`total`/`key` once, `paginate` the server's paging parameters (`limit`, `direction`) once, and `site:` is the page's own routes (`site:/auth/login`). Markup writes paths relative to `/api` (`src="/calendars"`, `data-sn-delete="/email/message?…"`, `<sn-sync src="/events/stream">`); every path, method, query parameter, body and `$result` is checked against the spec, and resources from the API take their type from it (no `type=` on them). TS code (`src/lib/api.ts`, actions' `ctx.request`) still writes full `/api/…` URLs and is not checked against the spec; test mocks name full URLs too.
- `src/sinter.types.ts`: the types `type="…"` may name (`<sn-event>` payloads, `call` resources, state); re-export each by name (no `export type *`).
- `src/components/`: sinter components (docs: `node_modules/@tiaforge-ui/sinter/docs/components.md`): `app-head` (a head component), `app-nav`, `nav-toggle`, event detail/form, thread, import, share menu, mail reader/compose/…; a `<name>.ts` beside one is its script (`month-fit`, `event-reminders`, `email-split`, `email-header-fit`, `email-hidden-watch`, `email-notifier`, `close-after`), which writes only the component's state and emits its declared events. The app's mail components are `email-*`: the `mail-` prefix belongs to sinter-mail (SN338). Types come from the generated `src/sinter.components.d.ts`. `src/layouts/app-layout.html`: the document of the pages with the rail. `src/islands/`: page-wide listeners, no elements (`common.ts`: `data-shortcut` keys, closing the drawer when a navigation starts (the Navigation API's `navigate`, `pageswap` for loads), the language pickers). `src/lib/`: shared TS (API, formatting, mail utils and types, PGP, i18n; `locale-path.ts`: page URLs code builds get the page's language prefix with `pagePath('/mail/?…')`, since the build rewrites only the links in the markup; an unprefixed one costs a redirect). `src/styles/`: `temper.ts` plus per-page Sass.
- Page wiring: an app page is `<app-layout data-sn-with="{ here: '/page/', title: 'Caley - Page' }" data-sn-state="{ … }">` (`title` is the whole tab title, `Caley` without one); a page whose `<main>` scrolls itself (settings, search, contacts) gives it `data-gutter` (the scrollbar's space stays reserved, so the centered column does not shift with classic scrollbars), a page of panes (mail, calendar) does not (it would keep an empty strip) → the page's resources and `<sn-sync>` → `<main>`, then `<script type="module" src="/src/main/<page>.ts">` (styles). The layout's tag takes DOM-event handlers only (they go onto `<body>`, page-wide keys); handle custom and component events on an element around their source, and put `data-sn-let` / `data-sn-persist` on an element inside. The layout declares `user` (read `user.map_url`, don't redeclare it); `<app-nav>` declares its own `<sn-sync>` for new-mail notifications; `<sn-sync>`s with the same URL share one connection, so pages declare their own only for their own events. `<nav-toggle>` opens the rail as a drawer on narrow screens. The login page and `/mail/view/` have their own document with `<app-head>`. Don't name state `page` (shadows `page()`).
- The mail page's URL holds what it shows (`src/lib/mail.ts` "The mail page's URL"): `?account_id=&mailbox=` or `?folder=` (the view), `uid` (the message open, in `account_id`'s `mailbox`: in a virtual folder the message's own), `page` (the list is offset-paginated with `paginate="{ numbered: 'offset', …, model: $url.page }"`; the search pages by keyset, and its response's `has_more` says whether there is a next page), and the search `q`, `sender` (repeated: a sinter list parameter, bound to the filter's checkboxes), `after`, `before`, `unread`. The search page's URL is `?q=&start=&end=&cal=` (`cal` repeated, bound to the calendar checkboxes: none checked searches every calendar). URL parameters are read and written through `$url` as sinter types them (a list for a `<select multiple>` or checkboxes with a `value`; a checkbox in a `data-sn-for` row needs a static `value=""` besides its bound one to count), not parsed by helpers. A view change is `navigate(viewHref(view))` (a history entry, dropping message, page and search); opening a message writes `$url.account_id/mailbox/uid`; below 1024px the reader overlay is "a message is open", so Back closes it. The page reads parameters by name (`$url.uid`), never `$url` whole: sinter follows a page's own links and `navigate()` in the page only when it can list the parameters the page reads (else it reloads).
- Conventions: components that declare a resource are used at page level, never inside `data-sn-if`/`data-sn-for`. A component sees only its props, its own state and the page's resources; it reports through declared events (`data-sn-emits`), never by writing page state. A dialog that edits a draft (`<event-form>`, `<import-dialog>`, `<email-compose>`, `<email-origins>`, `<email-account>`, `<email-folder>`) wraps the part of the page that opens it (its root has `display: contents`): an opener emits the draft (`emit('event-new', newEventDraft(…))`), the component's root handler sets its own `draft` and calls `openDialog`, so the form is rendered before the dialog opens. A dialog whose edits a navigation would lose (`<email-compose>`, `<event-form>`, the contact form) keeps an `edited` flag (set on `input`/`change`, cleared on `close`) and refuses leaving the page while it is set, with a note in the dialog's alert line: `data-sn-on-sn-navigate="edited && $event.preventDefault(); edited && (… = m.draft_leave())"` (the modal keeps the page's links out of reach, so this is Back and Forward). A DOM event runs only the nearest element's handler (custom events bubble on, DOM events do not), so a field with its own `data-sn-on-input`/`change` sets `edited` there too. Changes to mail messages reach both lists (`messages`, `results`) through the server's events (`email_updated`, `email_removed`), not through patches of another resource: `merge(item)` / `remove(item)` act on the resource the item came from. An optimistic change stays when the request succeeds and rolls back on error, so `data-sn-then` only writes what the server returned beyond it (`upsert(…, $result)`, `refresh(…)`), never the same change again. Custom events (`emit()`, islands' `CustomEvent`s, `sn-dismiss`) reach every ancestor that handles them, inner first, until a handler calls `$event.preventDefault()`; `page()` does nothing past the first or last page, so it needs no `$meta.x.hasNext &&` guard. Store functions and `page()` take the resource itself (`refresh(folders)`, `page(list, 'next')` through a let over `results`/`messages`), never its name as a string. A status text that comes and goes (a success, a wait, nothing found) sits in a live region that is always on the page, `<div class="live-region" role="status"><template data-sn-if="…">…</template></div>` (`.live-region` has no box, `app.scss`; sinter's SN545); an error is a `role="alert"`, which may be the top of its template. The expressions are checked with `strictNullChecks` (sinter follows `tsconfig.json`'s `strict`): a dialog's draft state that starts `null` is read inside a `<template data-sn-if="draft">`, not with `?.` everywhere or casts. A list's empty text is a `<template data-sn-else>` right after its `data-sn-for` (it shows once the list's resources have loaded, not while loading or after an error), where it is not inside a `<ul>`. A link built from data is `data-sn-href="/mail/?uid={…}"` (each value encoded, the page's language prefix added), not `data-sn-attr="{ href: … }"`. Persisted UI state uses `data-sn-persist="<key>"` (localStorage `sn:<key>`: `mail` (the view remembered for a visit without one in the URL), `calendar-mode` (the calendar's view; the old `calendar-view` held an object and is no longer read), `mail-widths` (the mail columns' widths), `nav-collapsed`, `event-default-calendar`, `caldav_base_url`); keep keys stable.
- Tests sit next to the markup (`<page>/index.test.ts`, `src/components/*.test.ts`) and use `@tiaforge-ui/sinter/testing`: `render(html, { ...app, requests: mockRequests({...}), actions, helpers, streams: { '/api/events/stream': mockStream() }, storage: memoryStorage() })`, then `flush()`/`settle()`. `app` (`src/test-components.ts`) passes the components and the layout (`render(…, { components, layouts })`); `render()` compiles as the build does (constants written in, decided branches); component scripts are left out (jsdom has no layout or ResizeObserver) unless a test passes them (`scripts: { 'close-after': scripts['close-after'] }`, or a stub class). Import HTML with `?raw`; `src/test-dom.ts` adds what jsdom lacks (modal dialogs, `hidePopover`, `scrollTo`, `commandfor` invokers). A navigation is recorded in `navigations` (absolute URLs, `replace …` for `{ replace: true }`).
- Styling uses temper (`@tiaforge-ui/temper`): semantic HTML plus `data-*` variants, light/dark automatic. The reference is `node_modules/@tiaforge-ui/temper/llms-full.txt`. App-specific CSS goes in `src/styles/*.scss` with `--tm-*` tokens, no hard-coded colors or outer margins.
- Icons are `data-icon="<lucide name>"`; the temper Vite plugin emits only names that appear as static strings in the markup.
- The temper Vite plugin runs `temper check` on every build (errors fail it). Check rendered pages with `npm run inspect` (backend in demo mode on :3000): it builds, logs in and runs temper-inspect on every page of the build the backend serves from `dist`; add `-- --sheet <dir>` for screenshots or `-- /mail/` for one page. The dev server also runs the audit in the page (`temper({ inspect: true })`: after load, hot updates, and on every dialog or popover opened, findings in its terminal with the source line), for a person working on the dev server; agents use `npm run inspect`.
- temper-inspect's known gaps (findings to read, not to fix in Caley): it finds a dialog's opener only through `commandfor`/`popovertarget`, and in the built page sinter's `openDialog('id')` handlers are compiled away. Caley opens dialogs with `openDialog()` (for readability), so those dialogs are opened directly, without the state a click would set: "opened directly … nearly empty" warnings are expected, and the contents of dialogs that are not empty go unchecked silently. Its "covered" check tests an element's box centre, so a wrapped inline link (the event location) is reported as covered by its own `<dd>`; on narrow screens with a message open, the reader overlay covers the mail list on purpose. `temper check` warns TM109 for `.reader`'s `data-grow` because it does not see through `<email-reader>`'s `display: contents` root. Pass URLs with state to reach more dialogs (`npm run inspect -- "/search/?q=a" "/mail/?account_id=demo-work&mailbox=INBOX&uid=3"`).
- i18n messages live in `frontend/messages/{locale}.json` (`project.inlang/settings.json`: `en` base, `de`), plain strings with `{param}`s; a missing `de` key falls back to `en` and is reported at build. Each language is its own page (`sinter({ localePrefix: true })`): `$locale` is a constant, assigning it is SN203. The language pickers (login page, settings) are `<select data-locale-switch>`s whose options' values are `localeUrl('de')` (this page in German); choosing one navigates there with the query (`src/islands/common.ts`), and that page sets the `locale` cookie. Tests render without a language, so the built-in `localeUrl` is not there: page tests that use it pass `src/lib/locale-path.ts`'s in `helpers`.
- `vite.config.ts` proxies `/api`, `/auth`, and `/logout` to `http://localhost:3000`; the dev server is usually `localhost:5173`. After a dependency change restart it with `npx vite --force`.
- Agents verify against the build, never the Vite dev server (its hot reloads navigate pages mid-check): `npm run build`, then (re)start the DEMO_MODE backend on `http://localhost:3000`, which serves `dist` with embedded data (it reads the pages at startup; WebAuthn works there too, since that origin is `APP_BASE_URL`). Agents working in parallel build into their own `--outDir` and serve it with `npx vite preview --outDir <dir> --port <port>`, which proxies `/api` like the dev server (no embedded data there: the browser requests it).

## Native App

`native/` is a Tauri 2 shell, not a second frontend: it bundles only a server picker (`native/ui/`, plain HTML/CSS/JS, no build step) and opens the chosen server's pages in its webview (`location.assign(origin + '/')`). The pages stay on the server's origin, so the session cookie, embedded data (`embed.rs`), swaps, SSE, OIDC redirects, the language redirects and the CSP work as in a browser, and the frontend needs no configurable API base. The backend forbids framing (`frame-ancestors 'none'`), so the shell navigates, never iframes.

- The picker keeps the servers in its own localStorage (`caley:servers`, most recent first) and opens the last one when the app starts; it comes back as `/?pick` (to choose, not to open the last one again) from Android's Back button on a server's first page and from the desktop menu's "Switch server" (Ctrl/Cmd+Shift+S, `add_menu` in `src-tauri/src/lib.rs`). Each server keeps its own cookies.
- Android specifics live in `src-tauri/gen/android` (generated by `android init`, committed; regenerate only for a Tauri update and keep these): `MainActivity.kt` pads the webview by the system bars, the display cutout and the keyboard (edge to edge is enforced for targetSdk 35+, and Caley's pages do not keep clear of the bars), with `caley_background` (`res/values*/colors.xml`, temper's `--tm-bg`) behind them, and handles Back: on the picker it leaves the app, on a server's pages it goes back in the webview while it can, then to the picker. It does not rely on the webview's history to hold the picker: the picker opens the last server while it loads, without a tap, and Chromium makes that navigation replace the picker's entry. Its Back handler is added again on every history change (`onWebViewHistoryChanged`), since Tauri's own (its app plugin's: back, else leave) is added after `onWebViewCreate` and the last one added is asked first.
- Adding a server runs the Rust command `probe_server` (`src-tauri/src/lib.rs`): `https://` unless typed, reduced to the origin, `GET /api/auth/config` must answer `{ oidc_enabled }`; the origin after redirects is the one saved (http → https). It runs in Rust because the picker may not read another origin (the backend sends no CORS headers). TLS as the backend's: rustls with ring and the webpki roots (no platform verifier, which needs JNI setup on Android).
- Only the picker has a capability (`capabilities/default.json`, `core:default`); the servers' pages get no Tauri IPC.
- What the servers' pages get on Android instead is the web APIs Android's WebView lacks, so Caley's frontend uses them as in a browser and knows nothing of the app (`gen/android/…/ShellBridge.kt`, `res/raw/shell_bridge.js`, injected at document start with androidx.webkit's `addDocumentStartJavaScript`, answered over the `caleyShell` web message listener, top frames only, never on the picker): `Notification` (Android notifications on the `caley` channel, the POST_NOTIFICATIONS permission asked by `requestPermission()`, `Notification.permission` written into the script since it must answer at once, re-injected on resume; a tap brings the app to front and fires the notification's click on the page that sent it, while that page lives), `navigator.share` (the share sheet, a file through the FileProvider's cache path, or text), and downloads: `<a download>` to a `blob:`/`data:` URL is saved to Downloads (MediaStore); the blob is kept from `URL.createObjectURL`, since the page's CSP (`connect-src 'self'`) does not let it be fetched; a download from the server goes to Android's download manager with the page's cookies. Messages are `{ ...args, id, type }`: an argument named `type` must not replace the message's.
- Passkeys on Android 11+ (`Authenticator.kt`): the WebView offers none for a server the app cannot name in advance (Digital Asset Links), so the bridge supplies `navigator.credentials.create/get` (for `publicKey`) and `PublicKeyCredential`, and the app is a platform authenticator: per credential an ECDSA P-256 key (ES256 assertions, what webauthn-rs verifies) and an HMAC-SHA256 key for the PRF extension (HMAC over SHA-256("WebAuthn PRF" ‖ 0 ‖ input), the spec's derivation), both in the Android Keystore (StrongBox when there is one), usable for 10 s after the screen lock (fingerprint, face or PIN) in the prompt each ceremony shows (`USE_BIOMETRIC`). The app writes the origin WebView reports into clientDataJSON and allows only an rpId that is that host or a parent domain of it (https or localhost); attestation `none`, user present and verified, a counter per credential (SharedPreferences `webauthn`). Neither the server nor Caley's vault code know it from another passkey: the phone joins a vault through the pending-device pairing (emoji) like any new passkey.
- Notifications while the app is closed (`Push.kt`, UnifiedPush connector 3.x): with a UnifiedPush distributor on the phone (e.g. ntfy), the app registers each server (the connector's instance is its origin) while that server's pages have Caley's mail notification switch on (`mail-notifications-enabled`; `shell_bridge.js` reports it with its hello and whenever the page writes it) and Android allows notifications. The VAPID key comes from `GET /api/notifications/vapid` and the endpoint with the device's Web Push keys (the connector makes them in the Keystore and decrypts what arrives) goes to `PUT /api/notifications/subscription`, both with the WebView's cookies, with the phone's timezone. The connector reports an endpoint only when it changes, so the app keeps the subscription (endpoint and the device's public keys, SharedPreferences `push`) and sends it again once per run (a new timezone, a server that lost it); switching off deletes it (the connector's `onUnregistered` is only the distributor's). `CaleyPushService` posts new mail and reminders (the event's time in the phone's format, its location) with the tags the page's own notifications use (they replace each other) and Caley's texts; a tap opens the message (`EXTRA_OPEN`: loaded at once, or after the first server page's hello when the activity or webview was just made). Without a distributor nothing changes: the pages notify while open. Testing against the demo backend: ntfy on the phone, the switch on in the rail, `POST /api/notifications/test`.
- Known gaps in the webview: passkeys made in the app stay on that phone (no sync; another device is paired like any new passkey) and the desktop shell has none (none in WebKitGTK; an iOS shell would need its own authenticator too, Associated Domains being fixed at build time); a page notification's tap after the app was closed only opens the app; the desktop shell has no bridge (its webviews' own notifications and downloads); WebKit webviews without the Navigation API load pages instead of swapping.
- Requirements: Node, Rust with the Android targets, JDK 17, the Android SDK (platform-tools, a platform, build-tools) and NDK, `JAVA_HOME`/`ANDROID_HOME`/`NDK_HOME`; Linux desktop builds also need WebKitGTK 4.1. `npm run android:init` once generates `src-tauri/gen/android` (committed), then `npm run android:dev` / `android:build`. Icons: `npx tauri icon app-icon.json` (in `native/`) writes every platform's from `app-icon.png` (the logo, shifted 2% left: centred by its outline the glyphs inside the C make it look right of centre, 4% looked left of centre) and the Android adaptive foreground `app-icon-fg.png` (the same on a larger transparent canvas, so the C fits the launcher's mask; the CLI ignores `android_fg_scale`), over white; the picker's `ui/logo.png` is the generated `icons/128x128@2x.png`. `identifier` (`app.caley`) is a placeholder until the app is published.
- Testing on a phone against a local backend: `adb reverse tcp:3000 tcp:3000` and add `http://localhost:3000`. The phone's localhost then reaches the PC: a secure context, so the session cookie (tower-sessions' default `Secure`) is kept, which it is not over `http://<LAN IP>`; debug builds allow cleartext http, release builds only https.

## Demo Mode

Set `DEMO_MODE=true` to start the backend in a fully self-contained demo mode — no external network calls are made. Use this to test the full UI end-to-end without any CalDAV, CardDAV, IMAP, or SMTP infrastructure.

- Uses an isolated `./data/demo.db` database (never touches the configured `DATABASE_PATH`).
- Accepts any credentials on the login page — always binds to the fixed demo user.
- Seeds realistic fixture data on startup (3 calendars, ~15 events, 8 contacts, 10 inbox messages). Uses `INSERT OR IGNORE` so data seeded in a previous run is preserved.
- CalDAV/CardDAV write ops (create/edit/delete event, contact) update the local DB only.
- SMTP sends are logged to stdout and silently discarded.
- Notifications are sent (`notifications.rs`, `reminders.rs`): the one network call demo mode makes, and only to the push endpoints a device registered. Demo mode syncs no mail, so it makes no `NewEmail`; `POST /api/notifications/test` sends a test, and an event with an alarm a few minutes ahead a reminder.
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

App shell:

```bash
cd native
npm install
npm run dev            # desktop
npm run android:init   # once
npm run android:dev    # device or emulator
npm run android:build  # APK
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
- JMAP tests: `src/jmap/fake.rs` is an in-memory JMAP server shaped like Stalwart's answers (the client, sync, operations, handlers and push run against it in `cargo test`); `src/jmap/stalwart_tests.rs` runs against a real Stalwart when `JMAP_TEST_URL` is set: `backend/scripts/stalwart-test.sh` starts `stalwartlabs/stalwart:v1.2.0` with podman or docker, finishes its bootstrap through the API, creates two users and an OAuth client (redirect URIs `http://127.0.0.1:3000/auth/jmap` and `/auth/oidc`), and writes `backend/.stalwart-test.env` (`set -a; . ./.stalwart-test.env; set +a; cargo test stalwart`; `scripts/stalwart-test.sh stop` removes it). Stalwart advertises `https://<hostname>/…` in its session, so the tests send that origin to the container (`JMAP_TEST_REWRITE`, read only under `cfg(test)`).
- OpenPGP private-key material should remain client-side decrypted only. Backend PGP endpoints store encrypted vault records and public metadata.
- Security headers in `main.rs` intentionally restrict script/image/frame behavior; update CSP carefully when adding frontend capabilities.
- Static file caching relies on hashed asset names: never serve unhashed files from `/assets/`, and keep HTML `no-cache`. A tab left open across a deploy can still request an old hashed chunk (openpgp, loaded lazily) that the new build no longer has: sinter reports it as `$error.stale` (nothing old is kept on the server).
- The app does not terminate TLS. Use a reverse proxy in production and keep `APP_BASE_URL` set to the public HTTPS origin, especially for OIDC and WebAuthn.
