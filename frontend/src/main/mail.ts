import '../styles/temper.ts';
import { initI18n } from '../lib/i18n.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import { mount } from 'svelte';
import MailApp from '../components/MailApp.svelte';

rememberCurrentAppUrl();

initI18n();
mount(MailApp, { target: document.getElementById('app')! });
