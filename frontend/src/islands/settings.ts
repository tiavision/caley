// Settings page islands: navigation after the account is deleted (sinter expressions cannot navigate).
import { clearReturnUrl } from '../lib/return-url.ts';

// The delete-account button emits this with the server's answer ({redirect}).
document.addEventListener('account-deleted', (e) => {
  clearReturnUrl();
  location.href = (e as CustomEvent<{ redirect?: string } | null>).detail?.redirect || '/calendar/';
});
