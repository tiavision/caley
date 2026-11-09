// Mail page (mail/index.html, sinter): styles and the mail islands; helpers, actions and types come through sinter.
import '../styles/temper.ts';
import '../styles/events.scss';
import '../styles/mail.scss';
import '../islands/mail.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import { preloadPgp } from '../lib/pgp/load.ts';

rememberCurrentAppUrl();
// openpgp is not part of the page (src/lib/pgp/load.ts): fetched in the background once the page is up.
preloadPgp();
