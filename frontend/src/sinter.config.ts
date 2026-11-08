// sinter app configuration, imported by every page before it mounts.
import { configure, onUnauthorized } from '@tiaforge-ui/sinter';
import { initI18n, getLocale, setLocale } from './lib/i18n.ts';
import { rememberCurrentAppUrl } from './lib/return-url.ts';
import './islands/common.ts';

initI18n();

configure({
  request: [
    // The same 401 handling as apiFetch (src/lib/api.ts): remember where we were, go to the login page.
    // On the login page itself a 401 from /api/user only means "not signed in": the user resource is null.
    onUnauthorized(async (req, res) => {
      if (location.pathname === '/') {
        return new URL(req.url).pathname === '/api/user' ? Response.json(null) : res;
      }
      rememberCurrentAppUrl();
      location.href = '/';
      return new Promise<Response>(() => {});
    }),
  ],
  // <sn-sync> reconnects only after this answers 2xx; a 401 goes to the login page.
  sessionProbe: '/api/user',
  locale: { get: getLocale, set: setLocale },
});
