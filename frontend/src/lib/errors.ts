// Errors of calls that may load openpgp (a lazy package, sinter({ lazy })): when the page is out of date (a deploy
// removed the chunk while the tab stayed open), say so instead of the action's own failure text. Pure; a helper.
import { m } from '@tiaforge-ui/sinter/messages';

/** "Reload the page" when `error` is a stale load, else `text`. */
export function loadErrorOr(error: { stale?: boolean }, text: string): string {
  return error.stale ? m.pgp_loadError() : text;
}
