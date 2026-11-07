import '../styles/temper.ts';
import { initI18n } from '../lib/i18n.ts';
import { redirectToPendingReturnUrl, rememberCurrentAppUrl } from '../lib/return-url.ts';
import { initRail } from './nav.ts';
import { mount } from 'svelte';
import CalendarApp from '../components/CalendarApp.svelte';

if (!redirectToPendingReturnUrl()) {
  rememberCurrentAppUrl();
}

initI18n();
initRail();
mount(CalendarApp, { target: document.getElementById('app')! });
