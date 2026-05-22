import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { initNav } from './nav.ts';
import { mount } from 'svelte';
import MailApp from '../components/MailApp.svelte';

initI18n();
initNav();
mount(MailApp, { target: document.getElementById('app')! });
