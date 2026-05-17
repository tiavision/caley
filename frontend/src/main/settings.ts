import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { initNav } from './nav.ts';
import { mount } from 'svelte';
import SettingsForm from '../components/SettingsForm.svelte';

initI18n();
initNav();
mount(SettingsForm, { target: document.getElementById('app')! });