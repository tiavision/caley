import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { initNav } from './nav.ts';
import { mount } from 'svelte';
import ContactsPage from '../components/ContactsPage.svelte';

initI18n();
initNav();
mount(ContactsPage, { target: document.getElementById('app')! });
