// sinter app configuration, imported by every page before it mounts.
import { configure, onUnauthorized } from '@tiaforge-ui/sinter';
import { retry } from '@tiaforge-ui/sinter/retry';
import { loginUrl } from './lib/return-url.ts';
import { withoutLocale } from './lib/locale-path.ts';
import './islands/common.ts';

configure({
  request: [
    // The same 401 handling as apiFetch (src/lib/api.ts): to the login page, which takes us back here after the sign-in.
    // On the login page itself a 401 from /api/user only means "not signed in": the user resource is null.
    onUnauthorized(async (req, res) => {
      if (withoutLocale(location.pathname) === '/') {
        return new URL(req.url).pathname === '/api/user' ? Response.json(null) : res;
      }
      location.href = loginUrl();
      return new Promise<Response>(() => {});
    }),
  ],
  // <sn-sync> reconnects only after this answers 2xx; a 401 goes to the login page. Retries ask it too.
  sessionProbe: '/api/user',
  // Sinter sends each request once. A load that hit a network error (a phone between networks) is tried
  // again a few times; so are the writes whose elements opt in with data-sn-retry="flags": the mail
  // flag PATCHes, which set a flag to a value and so may arrive twice. The backend ignores the
  // Idempotency-Key header (no write that needs it opts in).
  retry: retry({
    policies: {
      network: { backoff: ['1s', '10s'], retries: 3 },
      flags: { backoff: ['1s', '30s'], maxAge: '5m' },
    },
    get: 'network',
  }),
  // Contact addresses link to the device's map app (src/actions/maps.ts); decrypted mail attachments are blob: links.
  urlSchemes: ['geo:', 'blob:'],
});
