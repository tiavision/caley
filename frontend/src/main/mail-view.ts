// Standalone message view (mail/view/index.html, sinter): one message in its own window.
import '../styles/temper.ts';
import '../styles/events.scss';
import '../styles/mail.scss';
import '../islands/mail.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import { preloadPgp } from '../lib/pgp/load.ts';

rememberCurrentAppUrl();
// openpgp is not part of the page (src/lib/pgp/load.ts): fetched in the background once the page is up.
preloadPgp();
