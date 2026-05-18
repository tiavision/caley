import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { mount } from 'svelte';
import LoginForm from '../components/LoginForm.svelte';

initI18n();
mount(LoginForm, { target: document.getElementById('app')! });
