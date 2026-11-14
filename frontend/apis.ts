// The APIs the pages call (sinter({ apis }), sinter's docs/apis.md), for the build (vite.config.ts) and the page
// tests (src/test-components.ts): the backend's JSON API under /api, described by openapi.json, which the backend
// generates from its routes (`UPDATE_OPENAPI=1 cargo test openapi` in backend/; the test fails while the file is
// stale). Pages write paths relative to it (`src="/calendars"`), checked against the spec with their query
// parameters, bodies and types; the page's own routes outside /api are `site:` (`site:/auth/login`).

/** `openapi`: the spec's file (the build) or the parsed document (tests). */
export function apis<T>(openapi: T) {
  return {
    main: {
      base: '/api',
      openapi,
      // What each list endpoint returns, said once (the types come from the spec). /contacts is read as two
      // lists (its contacts, its address books), so its resources say which.
      defaults: {
        'GET /events': { key: 'date_iso' },
        'GET /email/thread': { items: 'messages', key: 'account_id,mailbox,uid' },
        'GET /email/thread-events': { key: 'account_id,mailbox,uid' },
        'GET /email/identities': { items: 'identities', key: 'identity_id' },
        'GET /email/accounts': { items: 'accounts', key: 'account_id' },
        'GET /email/mailboxes': { items: 'mailboxes', key: 'account_id,name' },
        'GET /email/folders': { items: 'folders', key: 'folder_id' },
        'GET /email/messages': { items: 'messages', total: 'total', key: 'account_id,mailbox,uid' },
        'GET /email/search': { items: 'messages', key: 'account_id,mailbox,uid' },
        'GET /email/suggest-addrs': { items: 'addrs' },
        'GET /search': { key: 'account_id,calendar_href,uid,dtstart' },
        'GET /caldav/accounts': { items: 'accounts', key: 'account_id' },
        'GET /calendar/event-emails': { key: 'account_id,mailbox,uid' },
        'GET /calendars': { key: 'account_id,href' },
        'GET /pgp/keys': { key: 'fingerprint' },
      },
    },
    // the sign-in, outside /api (backend/src/local_auth.rs)
    site: { operations: { 'POST /auth/login': {} } },
  };
}
