// The script of <email-notifier>: listens to the page's server events while the component is rendered.
import { instance } from '@tiaforge-ui/sinter';
import { type NewMailNotification, showNewMailNotification } from '../lib/mail-notifications.ts';
import { EmailNotifierComponent } from './email-notifier.html';

export class EmailNotifier extends EmailNotifierComponent {
  #off: (() => void) | null = null;

  connected() {
    this.#off = instance(document).on('new_email', (mail) => showNewMailNotification(mail as NewMailNotification));
  }

  disconnected() {
    this.#off?.();
    this.#off = null;
  }
}
