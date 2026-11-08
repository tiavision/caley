import '../styles/temper.ts';
import { initI18n } from '../lib/i18n.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import { mount } from 'svelte';
import SettingsForm from '../components/SettingsForm.svelte';

rememberCurrentAppUrl();

initI18n();
mount(SettingsForm, { target: document.getElementById('app')! });
