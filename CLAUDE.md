# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Caley** is a personal groupware web app. It is no longer only a CalDAV calendar viewer: the app now includes calendar management, CardDAV contacts, IMAP/SMTP email, full-text search, email identities, image privacy controls, and WebAuthn-backed OpenPGP key management.

The repository is a two-package monorepo with independent frontend and backend tooling:

- **`backend/`** — Rust 2024 Axum binary. Handles auth, CalDAV/CardDAV/IMAP/SMTP clients, SQLite persistence, encryption, sync tasks, SSE, and static frontend serving.
- **`frontend/`** — Vite static multi-page app: HTML pages compiled by sinter (`@tiaforge-ui/sinter`), temper CSS (`@tiaforge-ui/temper`, `data-*` attributes) + Sass for app-specific rules, sinter's built-in messages (inlang message files) for i18n.

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
- `pgp.rs`: WebAuthn credential enrollment/authentication, synced OpenPGP vault endpoints, public key storage, raw message access for PGP processing.
- `crypto.rs`: HKDF-SHA256 key derivation and AES-256-GCM encryption for stored secrets.
- `search.rs`: `SEARCH_MODE` parsing and FTS query/index text normalization.
- `sse.rs`: Server-sent events for calendar, contacts, and email updates.
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
- Encrypted IMAP accounts, mailboxes, messages, addresses, attachments, identities, virtual folders, image allow/block lists, unread state, and local `.eml` files under the database data directory.
- WebAuthn credentials.
- OpenPGP public keys.
- Synced encrypted OpenPGP private-key vault data, devices, and vault wraps.
- SQLite-backed session data.

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

- TypeScript strict mode. Pages are plain HTML compiled by sinter (`@tiaforge-ui/sinter`); the reference is `node_modules/@tiaforge-ui/sinter/llms.txt` (the short version; `llms-full.txt` is the whole one), and `npx sinter docs <topic | attribute | SNxxx>` prints one part of it, so look things up there instead of reading it again; `npx sinter render <page>.html --api http://localhost:3000` (or `--api mocks.json`, steps such as `--click '<selector>'`) runs a page on jsdom and prints its requests, markup and resources. Data lives in `<sn-resource>`s, live updates come from `<sn-sync src="/api/events/stream">` + `<sn-event>`, mutations use `data-sn-post/put/patch/delete` with `data-sn-then`/`catch`/`optimistic`, UI state uses `data-sn-state`/`let`/`model`, lists and conditionals use `<template data-sn-for/if>`, dialogs are native `<dialog>`s, shared markup is a component (`src/components/<name>.html`), the app pages share one layout (`src/layouts/app-layout.html`). Prefer plain HTML/CSS, then sinter directives, then helpers, then actions; component scripts only for layout measuring, foreign DOM or timers.
- `vite.config.ts`: `sinter({ typecheck: true, strict: true, embed: true, localePrefix: true, swap: true, lazy: ['openpgp'], components: ['@tiavision/sinter-mail'], … })` compiles every HTML entry and runs `sinter check` on `vite build`; strict mode (sinter's `docs/strict.md`) makes warnings fail like errors, in the build, `npx sinter check` and the dev server. Fix what a warning reports; keep a code a warning (`strict: { allow: { SNxxx: 'reason' } }`) only when the user agrees. `package.json` `sideEffects` lists the modules that act on import (styles, `src/styles/temper.ts`, `src/islands/*.ts`, page entries, `src/sinter.config.ts`; component scripts are not among them): a new one of those must be added there, or the build drops it.
- First paint (sinter's, see its `llms.txt` "First paint" and "Languages in the URL"). Each page is built per language (`localePrefix`), its text already in the HTML: no language file, nothing hidden until rendered. Each built page has two classic scripts first in its head: `assets/sinter-head-<hash>.js` (shared: the early runtime) and the page's `assets/<lang>-<page>-head-<hash>.js`. Nothing holds the paint: a `MutationObserver` renders each binding outside templates that needs no data (nav labels, `aria-current`, persisted state such as `nav-collapsed`) as the parser inserts its element, and the bindings over embedded data (`embed.rs`), `data-sn-for` rows included. The page bundle is `async` and takes the rendered DOM over once the document is parsed. Embedded resources (literal or `$url`-only `src`: `/api/user`, `/api/calendars`, `/api/email/accounts`, …) arrive in the HTML; the early file requests those whose URL reads early state, persisted values, `$now` and helpers (the calendar's week events and month grid, the mail list); the runtime takes those responses, so keep such URLs free of data from other resources if they should load early. So keep page chrome such as the nav rail outside `data-sn-if`/`data-sn-for` templates (inside them it waits for the page bundle), keep helpers that early bindings call light (they are copied into the early file; the build prints each early file's size), and remember that page-bundle code (`src/islands/`, `src/sinter.config.ts`) may run before the body is parsed: no DOM access at module top level.
- Navigation (sinter's swaps, `swap: true`, its `docs/navigation.md` "Swaps"): a link, `navigate()`, Back or Forward to another page of the app fetches that page's HTML (the same request a load sends, embedded data included) and swaps it in through the Navigation API, without a page load; a page in another language (the language pickers), a form's navigation, `/logout` and browsers without the Navigation API load the page. The rail is kept across swaps (`data-sn-permanent="rail"` on `<app-nav>`'s `<nav>`): the same element, its scroll and collapsed state; the next page's code takes over its bindings (`aria-current`), and its resources and `<sn-sync>` start again (the event stream reconnects at each swap: sinter's `docs/proposals/stream-handover.md` would keep it). Module code runs once per document: per-page work goes in a listener for `sn-page` (`document`, after each page mounts, the first included), never in an inline or classic script (SN540). The pages that swap carry no Speculation Rules (the swap fetches the page itself), so the CSP (`main.rs`) is `script-src 'self'`: turning swaps off would bring sinter's inline Speculation Rules back, which need `'inline-speculation-rules'` there. A link about to be followed (hover 65 ms, focus, press) requests the target page's literal-URL resources ahead (`sinter({ prefetch: false })` / `<html data-sn-prefetch="off">` / `<sn-resource early="off">` opt out). Keep GET endpoints free of side effects. Handlers go elsewhere with sinter's `navigate(url, { replace })` (after a sign-in, after the account is deleted), not with an island or component script; URLs built in code get the language prefix with the helper `pagePath()`. A link to the page itself that changes only query parameters the page reads stays on the page (sinter's same-page links), and a `$url` write from a user's event adds a history entry; `data-sn-model="$url.x"` binds a field to a URL parameter (the search page's dates). temper turns on the cross-document view transitions (`@view-transition`, not with reduced motion); `app.scss` only names and animates the rail and `<main>`.
- `src/sinter.config.ts` (imported by every page): 401 → the login page with this page as `next` (`loginUrl()`, `src/lib/return-url.ts`; the server keeps it, `landing.rs`), `sessionProbe: '/api/user'`, retry policies (`src/lib/retry-policies.ts`, also configured by the mail page tests; `@tiaforge-ui/sinter/retry`: every GET is tried again after a network error, `network`; the mail flag PATCHes opt in with `data-sn-retry="flags"`; only idempotent writes may opt in, the backend ignores `Idempotency-Key`), allowed URL schemes, the page-wide listeners of `src/islands/common.ts`. sinter is the locale source (no `configure({ locale })`).
- `src/sinter.helpers.ts`: pure functions templates call by name. Messages are sinter's: `m.key()` in markup, `import { m } from '@tiaforge-ui/sinter/messages'` in TS (typed by the generated `src/sinter.messages.d.ts`; only `m.key`, never `m[key]`). Page tests import the helpers whole, so they and what they re-export stay light and pure: no openpgp/`lib/pgp/*`, no DOM side effects.
- Mail bodies: sinter-mail (`@tiavision/sinter-mail`; its guide is `node_modules/@tiavision/sinter-mail/docs/mail.md`). The reader parses an HTML body in a let, `email.parse(html, bodyImages(msg, b.cids, imageVersion))` (`email` is the package's helper namespace in `src/sinter.helpers.ts`; `bodyImages` in `src/lib/mail.ts`: remote images through `/api/email/proxy-image`, `cid:` images from the message's parts or a decrypted message's `cids`), and `<mail-body data-sn-prop="{ body: tree && mailBody.progressive(tree) }">` renders the tree as part of the page, a long one progressively (its first 200 nodes at once, the rest in steps after a paint; `mailBody.progressive`, `src/lib/mail-body.ts`, numbers a copy, since the parsed tree is frozen): no iframe, no sanitizer (the parser keeps only what is safe, styles resolved and inline), `tree.images.origins` are the remote image origins. sinter-mail is not lazy: `<mail-body>`'s progressive steps are a component script of the package, which a lazy package cannot have in the page bundle (SN325). So the parser is in the mail pages' bundle, and `<email-reader>` (and compose's `<mail-body>`) carry `data-sn-early="off"`, keeping it and sinter-mail's components out of the early script. `.mail` (`mail.scss`) gives back the browser's defaults where mails count on them (tables, images, content-box sizing, block spacing) and leaves the rest to temper, and an HTML body takes the app's colours (dark mode included) unless it sets colours of its own without saying it works on a dark page (`keepsLight`): then it keeps a light canvas (`.mail-light`). A mail says so with its `color-scheme` (meta or property) or with `prefers-color-scheme: dark` rules (sinter-mail's `tree.schemes`); the colours such rules change are `light-dark()` in the tree and follow the page's scheme. Page tests get sinter-mail's components and its step script (`mail-steps`, without which a progressive body renders nothing) from `app` (`src/test-components.ts`).
- `src/sinter.actions.ts`: one namespace per module in `src/actions/` (`mail`, `ics`, `pgpVault`, `pgpKeys`, `notify`, `share`, `maps`) for imperative work (crypto, files, MIME), called with `data-sn-call="ns.fn(…)"` or `<sn-resource call>`. Actions return data and never touch the DOM or sinter state. openpgp is a lazy package (`sinter({ lazy: ['openpgp'] })` in `vite.config.ts`, sinter's `docs/lazy.md`): `src/lib/pgp/*` and the actions import it as usual, sinter loads it on the first call. Every call of an openpgp export must be awaited or returned; `new`, `instanceof`, `openpgp.enums`/`config` and helpers reaching it are SN324, and openpgp in a page bundle through another package is SN325. The mail pages load it once they have taken over: `data-sn-import="openpgp"` on `<email-reader>`'s root; settings and calendar on first use. A failed load is a `SinterError` (`stale` when a deploy removed the chunk): a catch on a call that may reach openpgp wraps its own text in the helper `loadErrorOr($error, …)` (`src/lib/errors.ts`: `m.pgp_loadError()` for a stale page), and `lib/pgp` and the actions let a `SinterError` through instead of turning it into their own error (a failed load is not a message that failed to decrypt or a bad signature).
- APIs: `frontend/apis.ts` is `sinter({ apis })` for the build and the page tests (`app` in `src/test-components.ts` passes it with the parsed spec): the API `main` is `/api` with `openapi.json`, `defaults` say each list endpoint's `items`/`total`/`key` once, and `site:` is the page's own routes (`site:/auth/login`). Markup writes paths relative to `/api` (`src="/calendars"`, `data-sn-delete="/email/message?…"`, `<sn-sync src="/events/stream">`); every path, method, query parameter, body and `$result` is checked against the spec, and resources from the API take their type from it (no `type=` on them). TS code (`src/lib/api.ts`, actions' `ctx.request`) still writes full `/api/…` URLs and is not checked against the spec; test mocks name full URLs too.
- `src/sinter.types.ts`: the types `type="…"` may name (`<sn-event>` payloads, `call` resources, state); re-export each by name (no `export type *`).
- `src/components/`: sinter components (docs: `node_modules/@tiaforge-ui/sinter/docs/components.md`): `app-head` (a head component), `app-nav`, `nav-toggle`, event detail/form, thread, import, share menu, mail reader/compose/…; a `<name>.ts` beside one is its script (`month-fit`, `event-reminders`, `email-split`, `email-header-fit`, `email-hidden-watch`, `email-notifier`, `close-after`), which writes only the component's state and emits its declared events. The app's mail components are `email-*`: the `mail-` prefix belongs to sinter-mail (SN338). Types come from the generated `src/sinter.components.d.ts`. `src/layouts/app-layout.html`: the document of the pages with the rail. `src/islands/`: page-wide listeners, no elements (`common.ts`: `data-shortcut` keys, closing the drawer when a navigation starts (the Navigation API's `navigate`, `pageswap` for loads), the language pickers). `src/lib/`: shared TS (API, formatting, mail utils and types, PGP, i18n; `locale-path.ts`: page URLs code builds get the page's language prefix with `pagePath('/mail/?…')`, since the build rewrites only the links in the markup; an unprefixed one costs a redirect). `src/styles/`: `temper.ts` plus per-page Sass.
- Page wiring: an app page is `<app-layout data-sn-prop="{ here: '/page/', title: 'Caley - Page' }" data-sn-state="{ … }">` (`title` is the whole tab title, `Caley` without one); a page whose `<main>` scrolls itself (settings, search, contacts) gives it `data-gutter` (the scrollbar's space stays reserved, so the centered column does not shift with classic scrollbars), a page of panes (mail, calendar) does not (it would keep an empty strip) → the page's resources and `<sn-sync>` → `<main>`, then `<script type="module" src="/src/main/<page>.ts">` (styles). The layout's tag takes DOM-event handlers only (they go onto `<body>`, page-wide keys); handle custom and component events on an element around their source, and put `data-sn-let` / `data-sn-persist` on an element inside. The layout declares `user` (read `user.map_url`, don't redeclare it); `<app-nav>` declares its own `<sn-sync>` for new-mail notifications; `<sn-sync>`s with the same URL share one connection, so pages declare their own only for their own events. `<nav-toggle>` opens the rail as a drawer on narrow screens. The login page and `/mail/view/` have their own document with `<app-head>`. Don't name state `page` (shadows `page()`).
- The mail page's URL holds what it shows (`src/lib/mail.ts` "The mail page's URL"): `?account_id=&mailbox=` or `?folder=` (the view), `uid` (the message open, in `account_id`'s `mailbox`: in a virtual folder the message's own), `page` (the list is offset-paginated with `page-model="$url.page"`), and the search `q`, `sender` (repeated: a sinter list parameter, bound to the filter's checkboxes), `after`, `before`, `unread`. The search page's URL is `?q=&start=&end=&cal=` (`cal` repeated, bound to the calendar checkboxes: none checked searches every calendar). URL parameters are read and written through `$url` as sinter types them (a list for a `<select multiple>` or checkboxes with a `value`; a checkbox in a `data-sn-for` row needs a static `value=""` besides its bound one to count), not parsed by helpers. A view change is `navigate(viewHref(view))` (a history entry, dropping message, page and search); opening a message writes `$url.account_id/mailbox/uid`; below 1024px the reader overlay is "a message is open", so Back closes it. The page reads parameters by name (`$url.uid`), never `$url` whole: sinter follows a page's own links and `navigate()` in the page only when it can list the parameters the page reads (else it reloads).
- Conventions: components that declare a resource are used at page level, never inside `data-sn-if`/`data-sn-for`. A component sees only its props, its own state and the page's resources; it reports through declared events (`data-sn-emits`), never by writing page state. A dialog that edits a draft (`<event-form>`, `<import-dialog>`, `<email-compose>`, `<email-origins>`, `<email-account>`, `<email-folder>`) wraps the part of the page that opens it (its root has `display: contents`): an opener emits the draft (`emit('event-new', newEventDraft(…))`), the component's root handler sets its own `draft` and calls `showModal`, so the form is rendered before the dialog opens. A dialog whose edits a navigation would lose (`<email-compose>`, `<event-form>`, the contact form) keeps an `edited` flag (set on `input`/`change`, cleared on `close`) and refuses leaving the page while it is set, with a note in the dialog's alert line: `data-sn-on-navigate="edited && $event.preventDefault(); edited && (… = m.draft_leave())"` (the modal keeps the page's links out of reach, so this is Back and Forward). A DOM event runs only the nearest element's handler (custom events bubble on, DOM events do not), so a field with its own `data-sn-on-input`/`change` sets `edited` there too. Changes to mail messages reach both lists (`messages`, `results`) through the server's events (`email_updated`, `email_removed`), not through patches of another resource: `patch(item)` / `remove(item)` act on the resource the item came from. An optimistic change stays when the request succeeds and rolls back on error, so `data-sn-then` only writes what the server returned beyond it (`upsert(…, $result)`, `refresh(…)`), never the same change again. Custom events (`emit()`, islands' `CustomEvent`s, `dismiss`) reach every ancestor that handles them, inner first, until a handler calls `$event.preventDefault()`; `page()` does nothing past the first or last page, so it needs no `$meta.x.hasNext &&` guard. Persisted UI state uses `data-sn-persist="<key>"` (localStorage `sn:<key>`: `mail` (the view remembered for a visit without one in the URL), `calendar-mode` (the calendar's view; the old `calendar-view` held an object and is no longer read), `mail-widths` (the mail columns' widths), `nav-collapsed`, `event-default-calendar`, `caldav_base_url`); keep keys stable.
- Tests sit next to the markup (`<page>/index.test.ts`, `src/components/*.test.ts`) and use `@tiaforge-ui/sinter/testing`: `render(html, { ...app, requests: mockRequests({...}), actions, helpers, streams: { '/api/events/stream': mockStream() }, storage: memoryStorage() })`, then `flush()`/`settle()`. `app` (`src/test-components.ts`) passes the components and the layout (`render(…, { components, layouts })`); `render()` compiles as the build does (constants written in, decided branches); component scripts are left out (jsdom has no layout or ResizeObserver) unless a test passes them (`scripts: { 'close-after': scripts['close-after'] }`, or a stub class). Import HTML with `?raw`; `src/test-dom.ts` adds what jsdom lacks (modal dialogs, `hidePopover`, `scrollTo`, `commandfor` invokers). A navigation is recorded in `navigations` (absolute URLs, `replace …` for `{ replace: true }`).
- Styling uses temper (`@tiaforge-ui/temper`): semantic HTML plus `data-*` variants, light/dark automatic. The reference is `node_modules/@tiaforge-ui/temper/llms-full.txt`. App-specific CSS goes in `src/styles/*.scss` with `--tm-*` tokens, no hard-coded colors or outer margins.
- Icons are `data-icon="<lucide name>"`; the temper Vite plugin emits only names that appear as static strings in the markup.
- The temper Vite plugin runs `temper check` on every build (errors fail it). Check rendered pages with `npm run inspect` (backend in demo mode on :3000): it builds, logs in and runs temper-inspect on every page of the build the backend serves from `dist`; add `-- --sheet <dir>` for screenshots or `-- /mail/` for one page. The dev server also runs the audit in the page (`temper({ inspect: true })`: after load, hot updates, and on every dialog or popover opened, findings in its terminal with the source line), for a person working on the dev server; agents use `npm run inspect`.
- temper-inspect's known gaps (findings to read, not to fix in Caley): it finds a dialog's opener only through `commandfor`/`popovertarget`, and in the built page sinter's `showModal('id')` handlers are compiled away. Caley opens dialogs with `showModal()` (the sinter variant, for readability), so those dialogs are opened directly, without the state a click would set: "opened directly … nearly empty" warnings are expected, and the contents of dialogs that are not empty go unchecked silently. Its "covered" check tests an element's box centre, so a wrapped inline link (the event location) is reported as covered by its own `<dd>`; on narrow screens with a message open, the reader overlay covers the mail list on purpose. `temper check` warns TM109 for `.reader`'s `data-grow` because it does not see through `<email-reader>`'s `display: contents` root. Pass URLs with state to reach more dialogs (`npm run inspect -- "/search/?q=a" "/mail/?account_id=demo-work&mailbox=INBOX&uid=3"`).
- i18n messages live in `frontend/messages/{locale}.json` (`project.inlang/settings.json`: `en` base, `de`), plain strings with `{param}`s; a missing `de` key falls back to `en` and is reported at build. Each language is its own page (`sinter({ localePrefix: true })`): `$locale` is a constant, assigning it is SN203. The language pickers (login page, settings) are `<select data-locale-switch>`s whose options' values are `localeUrl('de')` (this page in German); choosing one navigates there with the query (`src/islands/common.ts`), and that page sets the `locale` cookie. Tests render without a language, so the built-in `localeUrl` is not there: page tests that use it pass `src/lib/locale-path.ts`'s in `helpers`.
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
