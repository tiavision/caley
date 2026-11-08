// `maps` actions: open an address in the device's map app (`geo:` URL), or in the web map of the user's
// settings (`map_url` with a `{q}` placeholder) when no app took the link within half a second.
import type { CallContext } from '@tiaforge-ui/sinter';

const DEFAULT_MAP_URL = 'https://www.openstreetmap.org/search?query={q}';

/** `geo:` URL for an address query (pure; also a sinter helper for the link's href). */
export function geoUrl(query: string): string {
  return `geo:0,0?q=${encodeURIComponent(query)}`;
}

/** Follows the `geo:` link; opens `mapUrl` in a new tab if the page kept focus (no map app answered). */
export function open(query: string, mapUrl: string | null | undefined, ctx: CallContext): Promise<void> {
  const encoded = encodeURIComponent(query);
  let appOpened = false;
  const onBlur = () => { appOpened = true; };
  const onVisibility = () => { if (document.hidden) appOpened = true; };
  window.addEventListener('blur', onBlur, { once: true });
  document.addEventListener('visibilitychange', onVisibility, { once: true });
  window.location.href = geoUrl(query);
  return new Promise((resolve) => {
    setTimeout(() => {
      window.removeEventListener('blur', onBlur);
      document.removeEventListener('visibilitychange', onVisibility);
      if (!appOpened && !ctx.signal.aborted) window.open((mapUrl || DEFAULT_MAP_URL).replace('{q}', encoded), '_blank');
      resolve();
    }, 500);
  });
}
