// Login page islands: navigation and the return-URL bookkeeping of src/lib/return-url.ts.
import { markAuthReturnPending, redirectToReturnUrl } from '../lib/return-url.ts';

/** `<auth-return>`: rendered once someone is signed in; goes to the page they came from (or the calendar). */
customElements.define('auth-return', class extends HTMLElement {
  connectedCallback() {
    redirectToReturnUrl();
  }
});

// The single sign-on link emits this before the browser follows it: come back here after the IdP.
document.addEventListener('auth-return-pending', markAuthReturnPending);
