import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { mount } from 'svelte';
import LoginPage from '../components/LoginPage.svelte';

initI18n();
mount(LoginPage, { target: document.getElementById('app')! });
