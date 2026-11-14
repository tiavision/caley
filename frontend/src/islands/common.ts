// Browser work every page shares (imported by src/sinter.config.ts): keyboard shortcuts, the drawer on page swap and the language pickers.
// Components with a script (src/components/*.ts) hold the per-element work: <mail-notifier>, <close-after>, …
import { shortcutMatches, typingTarget } from '../lib/shortcuts.ts';

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
 * The language pickers (login page, settings): each language is its own page (`localePrefix`, `/de/…`), so a
 * picker is a `<select data-locale-switch>` whose options' values are `localeUrl(…)`; choosing one goes there,
 * with this page's query and fragment (sinter expressions cannot navigate). That page records the choice.
 */
document.addEventListener('change', (e) => {
  const select = e.target;
  if (!(select instanceof HTMLSelectElement) || !select.matches('[data-locale-switch]')) return;
  location.assign(select.value + location.search + location.hash);
});
