// sinter app configuration, imported by every page before it mounts.
import { configure, onUnauthorized } from '@tiaforge-ui/sinter';
import { getLocale, setLocale } from './lib/i18n.ts';
import { initLanguage } from './lib/language.ts';
import { rememberCurrentAppUrl } from './lib/return-url.ts';
import './islands/common.ts';

// The language: the same pick as the early file's (src/sinter.early.ts), applied to the page bundle's own copy of
// paraglide, as the early file is bundled on its own and shares no module instance with the page.
initLanguage();

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
  // Contact addresses link to the device's map app (src/actions/maps.ts); decrypted mail attachments are blob: links.
  urlSchemes: ['geo:', 'blob:'],
});
