// `share` actions: partials/share-menu.html shares a ShareItem (src/lib/share.ts) through the system share
// sheet (touch devices with the Web Share API) or offers it as a download or as a mail attachment.
import type { CallContext } from '@tiaforge-ui/sinter';
import { composeEmailWithAttachment, downloadFile, type ShareItem } from '../lib/share.ts';

async function content(item: ShareItem, ctx: CallContext): Promise<string> {
  return String(await ctx.request(item.url));
}

/** Whether the share button opens the system share sheet (touch screen and Web Share API) instead of the menu. */
export function native(): boolean {
  return window.matchMedia('(pointer: coarse)').matches && typeof navigator.share === 'function';
}

/** Opens the system share sheet with the file (or only the title where files cannot be shared). */
export async function send(item: ShareItem, ctx: CallContext): Promise<void> {
  const file = new File([await content(item, ctx)], item.filename, { type: item.type });
  try {
    await navigator.share(navigator.canShare?.({ files: [file] }) ? { title: item.title, files: [file] } : { title: item.title });
  } catch (err) {
    if (!(err instanceof Error && err.name === 'AbortError')) throw err;
  }
}

/** Saves the file. */
export async function download(item: ShareItem, ctx: CallContext): Promise<void> {
  downloadFile(item.filename, await content(item, ctx), item.type);
}

/** Opens the mail composer with the file attached. */
export async function email(item: ShareItem, ctx: CallContext): Promise<void> {
  composeEmailWithAttachment(item.title, item.filename, item.type, await content(item, ctx));
}
