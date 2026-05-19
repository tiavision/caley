import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { initNav } from './nav.ts';
import { mount } from 'svelte';
import SearchPage from '../components/SearchPage.svelte';

initI18n();
initNav();
mount(SearchPage, { target: document.getElementById('app')! });
