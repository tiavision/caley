import '../styles/temper.ts';
import { initI18n } from '../lib/i18n.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import { mount } from 'svelte';
import ContactsPage from '../components/ContactsPage.svelte';

rememberCurrentAppUrl();

initI18n();
mount(ContactsPage, { target: document.getElementById('app')! });
