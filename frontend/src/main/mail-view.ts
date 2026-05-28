import '../styles/main.scss';
import { initI18n } from '../lib/i18n.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import { mount } from 'svelte';
import MailApp from '../components/MailApp.svelte';

rememberCurrentAppUrl();

initI18n();

const params = new URLSearchParams(window.location.search);
const uid = Number(params.get('uid'));
const accountId = params.get('account_id') ?? '';
const mailbox = params.get('mailbox') ?? '';
const initialMessage = accountId && mailbox && Number.isInteger(uid) && uid > 0
  ? { accountId, mailbox, uid }
  : null;

mount(MailApp, {
  target: document.getElementById('app')!,
  props: { standalone: true, initialMessage },
});
