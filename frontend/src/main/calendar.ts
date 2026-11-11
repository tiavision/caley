// Calendar page (calendar/index.html, sinter): styles; helpers, actions and types come through sinter.
import '../styles/temper.ts';
import '../styles/events.scss';
import '../styles/calendar.scss';
import { redirectToPendingReturnUrl, rememberCurrentAppUrl } from '../lib/return-url.ts';

if (!redirectToPendingReturnUrl()) {
  rememberCurrentAppUrl();
}
