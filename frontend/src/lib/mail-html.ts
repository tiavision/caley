import DOMPurify from 'dompurify';

export interface MailHtmlContext {
  accountId?: string;
  mailbox?: string;
  uid?: number;
}

const imageDataPattern = /^data:image\/(?!svg\b)[a-z0-9.+-]+;base64,/i;
const allowedTags = [
  'a', 'abbr', 'b', 'blockquote', 'br', 'cite', 'code', 'col', 'colgroup',
  'del', 'div', 'dl', 'dt', 'dd', 'em', 'font', 'h1', 'h2', 'h3', 'h4',
  'h5', 'h6', 'hr', 'i', 'img', 'ins', 'li', 'ol', 'p', 'pre', 's',
  'small', 'span', 'strike', 'strong', 'sub', 'sup',
  'table', 'tbody', 'td', 'tfoot', 'th', 'thead', 'tr', 'u', 'ul',
];
const allowedAttrs = [
  'align', 'alt', 'bgcolor', 'border', 'cellpadding', 'cellspacing', 'class',
  'color', 'colspan', 'dir', 'face', 'height', 'href', 'rel', 'rowspan',
  'size', 'span', 'src', 'style', 'target', 'title', 'valign', 'width',
];

function encode(value: string): string {
  return encodeURIComponent(value);
}

function proxiedRemoteImage(src: string): string {
  return `/api/email/proxy-image?url=${encode(src)}`;
}

function proxiedCidImage(src: string, context: MailHtmlContext): string | null {
  const cid = src.slice(4);
  if (!context.accountId || !context.mailbox || !context.uid || !cid) return null;
  return `/api/email/proxy-image?account_id=${encode(context.accountId)}&mailbox=${encode(context.mailbox)}&uid=${context.uid}&cid=${encode(cid)}`;
}

function cleanStyle(value: string): string {
  return value
    .split(';')
    .map(decl => decl.trim())
    .filter(decl => decl && !/\burl\s*\(/i.test(decl))
    .filter(decl => !/\bexpression\s*\(/i.test(decl))
    .filter(decl => !/^\s*(behavior|-moz-binding)\s*:/i.test(decl))
    .filter(decl => !/^\s*position\s*:\s*fixed\b/i.test(decl))
    .join('; ');
}

export function prepareEmailHtml(html: string, context: MailHtmlContext = {}): string {
  const hook = (node: Element) => {
    if (!(node instanceof Element)) return;

    if (node instanceof HTMLAnchorElement) {
      const href = node.getAttribute('href');
      const lower = href?.trimStart().toLowerCase() ?? '';
      if (href && (lower.startsWith('http://') || lower.startsWith('https://') || lower.startsWith('mailto:'))) {
        node.setAttribute('target', '_blank');
        node.setAttribute('rel', 'noopener noreferrer');
      } else {
        node.removeAttribute('href');
        node.removeAttribute('target');
        node.removeAttribute('rel');
      }
    } else {
      node.removeAttribute('href');
    }

    if (node instanceof HTMLImageElement) {
      const src = node.getAttribute('src') ?? '';
      const lower = src.trimStart().toLowerCase();
      if (lower.startsWith('/api/email/proxy-image?')) {
        // Already prepared by a previous render path or from older cached mail.
      } else if (lower.startsWith('http://') || lower.startsWith('https://')) {
        node.setAttribute('src', proxiedRemoteImage(src));
      } else if (lower.startsWith('cid:')) {
        const proxied = proxiedCidImage(src, context);
        if (proxied) node.setAttribute('src', proxied);
        else node.removeAttribute('src');
      } else if (!imageDataPattern.test(src)) {
        node.removeAttribute('src');
      }
    } else {
      node.removeAttribute('src');
    }
    node.removeAttribute('srcset');

    const style = node.getAttribute('style');
    if (style !== null) {
      const clean = cleanStyle(style);
      if (clean) node.setAttribute('style', clean);
      else node.removeAttribute('style');
    }
  };

  DOMPurify.addHook('afterSanitizeAttributes', hook);
  try {
    return DOMPurify.sanitize(html, {
      FORCE_BODY: true,
      ALLOWED_TAGS: allowedTags,
      ALLOWED_ATTR: allowedAttrs,
      ALLOW_DATA_ATTR: false,
      ADD_URI_SAFE_ATTR: ['src'],
      ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto|cid):|data:image\/(?!svg\b)|\/api\/email\/proxy-image\?)/i,
    });
  } finally {
    DOMPurify.removeHook('afterSanitizeAttributes');
  }
}

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

export function imageOriginsFromHtml(html: string): string[] {
  const found = new Set<string>();
  const re = /\/api\/email\/proxy-image\?url=([^"&\s]+)/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(html)) !== null) {
    try {
      const decoded = decodeURIComponent(match[1]);
      const url = new URL(decoded);
      found.add(`${url.protocol}//${url.host}`);
    } catch {
      // Ignore malformed legacy proxy URLs.
    }
  }
  return [...found];
}
