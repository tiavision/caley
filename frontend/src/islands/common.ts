// Browser work every page shares (imported by src/sinter.config.ts): keyboard shortcuts, the drawer on page swap and the language pickers.
// Components with a script (src/components/*.ts) hold the per-element work: <email-notifier>, <close-after>, …
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
 * its links navigates while it is open. The old page is captured for the view transition, so the open drawer
 * would be in the snapshot and fade out over the new page. Hide it first, without its slide-out transition, so
 * the new page starts with the drawer closed. Pages swap (`sinter({ swap: true })`) and the rail is kept
 * (`data-sn-permanent`), so it would stay open: close it when a navigation starts (the Navigation API's
 * `navigate`, before sinter fetches the next page). `pageswap` covers page loads (no Navigation API, or a page
 * that does not swap). Its transition comes back once the next page is in (`sn-page`).
 */
function closeDrawer(): void {
  const rail = document.querySelector<HTMLElement>('body[data-tm-shell] > nav[data-tm-rail][popover]');
  if (!rail?.matches(':popover-open')) return;
  rail.style.transition = 'none';
  rail.hidePopover();
  void getComputedStyle(rail).display;   // hidden now, before the transition is allowed again
}
window.addEventListener('pageswap', closeDrawer);
(window as { navigation?: EventTarget }).navigation?.addEventListener('navigate', (e) => {
  const to = (e as Event & { destination?: { url: string } }).destination?.url;
  if (to && new URL(to).pathname !== location.pathname) closeDrawer();
});
document.addEventListener('sn-page', () => {
  document.querySelector<HTMLElement>('body[data-tm-shell] > nav[data-tm-rail]')?.style.removeProperty('transition');
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
