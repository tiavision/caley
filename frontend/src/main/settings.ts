import '../styles/temper.ts';
import { initI18n } from '../lib/i18n.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import { initRail } from './nav.ts';
import { mount } from 'svelte';
import SettingsForm from '../components/SettingsForm.svelte';

rememberCurrentAppUrl();

initI18n();
initRail();
mount(SettingsForm, { target: document.getElementById('app')! });
