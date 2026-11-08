import '../styles/temper.ts';
import { initI18n } from '../lib/i18n.ts';
import { redirectToPendingReturnUrl, rememberCurrentAppUrl } from '../lib/return-url.ts';
import { mount } from 'svelte';
import CalendarApp from '../components/CalendarApp.svelte';

if (!redirectToPendingReturnUrl()) {
  rememberCurrentAppUrl();
}

initI18n();
mount(CalendarApp, { target: document.getElementById('app')! });
