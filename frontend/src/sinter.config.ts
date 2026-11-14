// sinter app configuration, imported by every page before it mounts.
import { configure, onUnauthorized } from '@tiaforge-ui/sinter';
import { retryPolicies } from './lib/retry-policies.ts';
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
  // Loads after a network error, and the writes that opt in (data-sn-retry="flags"), are tried again.
  retry: retryPolicies,
  // Contact addresses link to the device's map app (src/actions/maps.ts); decrypted mail attachments are blob: links.
  urlSchemes: ['geo:', 'blob:'],
});
