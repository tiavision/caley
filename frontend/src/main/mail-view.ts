// Standalone message view (mail/view/index.html, sinter): one message in its own window.
import '../styles/temper.ts';
import '../styles/mail.scss';
import '../islands/mail.ts';
import { rememberCurrentAppUrl } from '../lib/return-url.ts';
import './mail-bridge.svelte.ts';

rememberCurrentAppUrl();
