import '../styles/temper.ts';
import { initI18n } from '../lib/i18n.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import { initRail } from './nav.ts';
import { mount } from 'svelte';
import SearchPage from '../components/SearchPage.svelte';

rememberCurrentAppUrl();

initI18n();
initRail();
mount(SearchPage, { target: document.getElementById('app')! });
