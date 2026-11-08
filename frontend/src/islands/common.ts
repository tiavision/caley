// Islands shared by every page (imported by src/sinter.config.ts): browser work sinter expressions cannot do.
import { instance } from '@tiaforge-ui/sinter';
import { getLocale, makeTl } from '../lib/i18n.ts';
import { type NewMailNotification, showNewMailNotification } from '../lib/mail-notifications.ts';

/**
 * `<mail-notifier>`: while connected, shows a desktop notification for each `new_email` server event
 * of the page's /api/events/stream <sn-sync>. The nav rail renders it when notifications are on.
 */
customElements.define('mail-notifier', class extends HTMLElement {
  #off: (() => void) | null = null;

  connectedCallback() {
    this.#off = instance(document).on('new_email', (mail) => {
      showNewMailNotification(mail as NewMailNotification, makeTl(getLocale()));
    });
  }

  disconnectedCallback() {
    this.#off?.();
    this.#off = null;
  }
});
