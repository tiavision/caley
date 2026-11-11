// Login page: the single sign-on link emits this before the browser follows it: come back here after the IdP.
// (<auth-return>, the redirect once signed in, is src/components/auth-return.ts.)
import { markAuthReturnPending } from '../lib/return-url.ts';

document.addEventListener('auth-return-pending', markAuthReturnPending);
