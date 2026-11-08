// Calendar page (calendar/index.html, sinter): styles and islands; helpers, actions and types come through sinter.
import '../styles/temper.ts';
import '../styles/events.scss';
import '../styles/calendar.scss';
import '../islands/calendar.ts';
import { redirectToPendingReturnUrl, rememberCurrentAppUrl } from '../lib/return-url.ts';

if (!redirectToPendingReturnUrl()) {
  rememberCurrentAppUrl();
}
