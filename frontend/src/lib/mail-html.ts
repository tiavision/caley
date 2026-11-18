// HTML mail bodies outside the reader: their text, the inline images they reference, a forward's inline images.
// The reader renders a body with sinter-mail (email.parse and <mail-body>), which keeps only what is safe to show:
// nothing here sanitizes, and nothing here is shown as HTML.

/** The text of an HTML body, for a decrypted message without a text part (replies quote it). */
export function htmlToPlainText(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.querySelectorAll('script, style, title, noscript').forEach(el => el.remove());
  doc.querySelectorAll('br, p, div, li, tr, h1, h2, h3, h4, h5, h6, blockquote, hr').forEach(el => {
    el.appendChild(doc.createTextNode('\n'));
  });
  return (doc.body.textContent ?? '')
    .replace(/\r\n?/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

/** The content ids of the inline images (`cid:`) an HTML body references. */
export function cidReferencesFromHtml(html: string): string[] {
  const found = new Set<string>();
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.querySelectorAll('img[src]').forEach(img => {
    const src = (img.getAttribute('src') ?? '').trim();
    if (src.toLowerCase().startsWith('cid:')) found.add(src.slice(4));
  });
  return [...found];
}

/** `html` with its inline images (`cid:…`) replaced by the URLs of `cids` (by content id); the others kept. */
export function withInlineImages(html: string, cids: Record<string, string>): string {
  let out = html;
  for (const [cid, url] of Object.entries(cids)) {
    out = out.replace(new RegExp(`cid:${cid.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`, 'gi'), url);
  }
  return out;
}
