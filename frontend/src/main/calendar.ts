import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { initNav } from './nav.ts';
import { mount } from 'svelte';
import CalendarApp from '../components/CalendarApp.svelte';

initI18n();
initNav();
mount(CalendarApp, { target: document.getElementById('app')! });
