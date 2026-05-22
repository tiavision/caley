import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { mount } from 'svelte';
import MailApp from '../components/MailApp.svelte';

initI18n();
mount(MailApp, { target: document.getElementById('app')! });
