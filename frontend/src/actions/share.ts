// `share` actions: src/components/share-menu.html shares a ShareItem (src/lib/share.ts) through the system share
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

/**
 * The file the share sheet gets, fetched ahead (a `call` resource of <share-menu>): browsers open the sheet only
 * right after the tap (Safari strictly), so send() must not wait for the network first.
 */
export async function prepare(item: ShareItem, ctx: CallContext): Promise<File> {
  return new File([await content(item, ctx)], item.filename, { type: item.type });
}

/**
 * Opens the system share sheet with `item`'s file: the one prepared ahead when it is this item's (nothing awaited
 * before the sheet), else fetched now (the browser may then refuse: the menu opens instead, `data-sn-catch`).
 */
export async function send(item: ShareItem, file: File | null, ctx: CallContext): Promise<void> {
  const ready = file && file.name === item.filename && file.type === item.type ? file : await prepare(item, ctx);
  try {
    await navigator.share(navigator.canShare?.({ files: [ready] }) ? { title: item.title, files: [ready] } : { title: item.title });
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
