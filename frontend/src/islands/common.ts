// Islands shared by every page (imported by src/sinter.config.ts): browser work sinter expressions cannot do.
import { instance } from '@tiaforge-ui/sinter';
import { type NewMailNotification, showNewMailNotification } from '../lib/mail-notifications.ts';
import { shortcutMatches, typingTarget } from '../lib/shortcuts.ts';

/**
 * `<mail-notifier>`: while connected, shows a desktop notification for each `new_email` server event
 * of the page's /api/events/stream <sn-sync>. The nav rail renders it when notifications are on.
 */
customElements.define('mail-notifier', class extends HTMLElement {
  #off: (() => void) | null = null;

  connectedCallback() {
    this.#off = instance(document).on('new_email', (mail) => {
      showNewMailNotification(mail as NewMailNotification);
    });
  }

  disconnectedCallback() {
    this.#off?.();
    this.#off = null;
  }
});

/**
 * `<close-after ms="1500">`: closes the dialog it is in after that delay (sinter expressions have no timers).
 * Render it with `data-sn-if` when the dialog is done, e.g. the import dialog's success message.
 */
customElements.define('close-after', class extends HTMLElement {
  #timer: ReturnType<typeof setTimeout> | undefined;

  connectedCallback() {
    this.#timer = setTimeout(() => this.closest('dialog')?.close(), Number(this.getAttribute('ms') ?? 0));
  }

  disconnectedCallback() {
    clearTimeout(this.#timer);
  }
});

/**
 * Keyboard shortcuts of visible controls: a keydown that matches a `data-shortcut` (src/lib/shortcuts.ts) clicks
 * that control, so the shortcut does exactly what the click does. Only enabled controls that are shown count; a
 * menu item counts while its menu's button is shown, so `r` works with the menu closed. With a dialog open, only
 * its own controls count. Keys typed into fields, and keys a handler already took, are left alone.
 */
export function clickShortcut(e: KeyboardEvent): void {
  if (e.defaultPrevented || e.isComposing || typingTarget(e.target)) return;
  const dialog = [...document.querySelectorAll('dialog[open]')].at(-1);
  const control = [...(dialog ?? document).querySelectorAll<HTMLElement>('[data-shortcut]')].find((el) =>
    shortcutMatches(el.dataset.shortcut ?? '', e) && (dialog || !el.closest('dialog')) && enabled(el) && shown(el));
  if (!control) return;
  e.preventDefault();
  control.click();
}

function enabled(el: HTMLElement): boolean {
  return !el.matches(':disabled') && el.getAttribute('aria-disabled') !== 'true';
}

function shown(el: HTMLElement): boolean {
  if (!el.checkVisibility) return !el.closest('[hidden]');   // jsdom
  if (el.checkVisibility()) return true;
  const menu = el.closest<HTMLElement>('[popover]');
  if (!menu || menu.matches(':popover-open')) return false;
  for (let n: HTMLElement | null = el; n && n !== menu; n = n.parentElement) if (getComputedStyle(n).display === 'none') return false;
  return [...document.querySelectorAll<HTMLElement>(`[popovertarget="${menu.id}"]`)].some((opener) => opener.checkVisibility());
}

document.addEventListener('keydown', clickShortcut);

/**
 * Page transitions (src/styles/app.scss): on narrow screens the nav rail is a popover drawer, and a tap on one of
 * its links navigates while it is open. The old page is captured for the cross-document view transition right
 * after `pageswap`, so the open drawer would be in the snapshot and fade out over the new page, where it is
 * closed. Hide it here first, without its slide-out transition, so the new page starts with the drawer closed.
 */
window.addEventListener('pageswap', () => {
  const rail = document.querySelector<HTMLElement>('body[data-shell] > nav[data-rail][popover]');
  if (!rail?.matches(':popover-open')) return;
  rail.style.transition = 'none';
  rail.hidePopover();
});

/**
 * Sets data-styled on <html> once every stylesheet in the head has applied and the early file has rendered; until
 * then src/styles/app.scss disables transitions. Firefox resolves styles before the head's stylesheets are in and
 * before the early file runs (Chromium holds rendering for both), so a transition declared in a sheet would run from
 * the user-agent default to the sheet's resting state as the sheet lands (the mail folder drawer slid closed on every
 * load), and the early file's bindings would animate from the unrendered state (the collapsed rail narrowed from
 * its full width). The style flush before the marker keeps those changes and the re-enabled transitions out of the
 * same style update. The page bundle is async, so a sheet may already be in; the early file may or may not have run.
 *
 * The early file's completion is sinter's internal handshake with its runtime (the runtime awaits the same promise
 * before it mounts): `globalThis[Symbol.for('sinter.early.page')]` is `{ started, done, finish }`, created by
 * whichever side comes first. Replace with a public signal once sinter has one; `load` is the fallback.
 */
interface EarlyPage { started: boolean; done: Promise<void>; finish: () => void }
function earlyRendered(): Promise<void> {
  const key = Symbol.for('sinter.early.page');
  const g = globalThis as unknown as Record<symbol, EarlyPage | undefined>;
  if (!g[key]) {
    let finish!: () => void;
    const done = new Promise<void>((resolve) => { finish = resolve; });
    g[key] = { started: false, done, finish };
  }
  const loaded = document.readyState === 'complete'
    ? Promise.resolve()
    : new Promise<void>((resolve) => window.addEventListener('load', () => resolve(), { once: true }));
  return Promise.race([g[key]!.done, loaded]);
}
function markStyled() {
  const sheets = Array.from(document.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]'))
    .filter((link) => !link.sheet)
    .map((link) => new Promise<void>((resolve) => {
      link.addEventListener('load', () => resolve(), { once: true });
      link.addEventListener('error', () => resolve(), { once: true });
    }));
  Promise.all([...sheets, earlyRendered()]).then(() => {
    // A layout flush: Firefox's getComputedStyle resolves only the queried element, not the whole document.
    void document.documentElement.getBoundingClientRect();
    document.documentElement.dataset.styled = '';
  });
}
if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', markStyled);
else markStyled();
