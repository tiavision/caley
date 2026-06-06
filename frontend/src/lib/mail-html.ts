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

    // Mirror valid numeric width/height HTML attributes into inline CSS so they
    // take effect with author-level specificity. Browsers may ignore presentational
    // hints when an invalid value like height="auto" is also present.
    if (node instanceof HTMLImageElement) {
      const wAttr = node.getAttribute('width');
      const hAttr = node.getAttribute('height');
      const currentStyle = node.getAttribute('style') ?? '';
      const additions: string[] = [];
      if (wAttr !== null) {
        const w = parseInt(wAttr, 10);
        if (!isNaN(w) && !/\bwidth\s*:/i.test(currentStyle)) additions.push(`width: ${w}px`);
      }
      if (hAttr !== null) {
        const h = parseInt(hAttr, 10);
        if (!isNaN(h) && !/\bheight\s*:/i.test(currentStyle)) additions.push(`height: ${h}px`);
      }
      if (additions.length) {
        node.setAttribute('style', currentStyle ? `${currentStyle}; ${additions.join('; ')}` : additions.join('; '));
      }
    }
  };

  DOMPurify.addHook('afterSanitizeAttributes', hook);
  try {
    return DOMPurify.sanitize(html, {
      FORCE_BODY: true,
      ALLOWED_TAGS: allowedTags,
      ALLOWED_ATTR: allowedAttrs,
      ALLOW_DATA_ATTR: false,
      // When ALLOWED_URI_REGEXP is set, DOMPurify validates every non-empty
      // attribute value against it — not just href/src. Presentational attributes
      // like width="150", height="auto", bgcolor="#eee" would be stripped because
      // their values don't look like URIs. Adding them to URI_SAFE_ATTRIBUTES
      // exempts them from that check.
      ADD_URI_SAFE_ATTR: [
        'src',
        'align', 'bgcolor', 'border', 'cellpadding', 'cellspacing', 'color',
        'colspan', 'dir', 'face', 'height', 'rel', 'rowspan', 'size', 'span',
        'target', 'valign', 'width',
      ],
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

function portableImageSource(src: string): string {
  const trimmed = src.trim();
  const lower = trimmed.toLowerCase();
  if (lower.startsWith('cid:') || lower.startsWith('http://') || lower.startsWith('https://') || imageDataPattern.test(trimmed)) {
    return src;
  }
  if (!lower.startsWith('/api/email/proxy-image?')) return src;

  try {
    const url = new URL(trimmed, 'https://caley.local');
    const cid = url.searchParams.get('cid');
    if (cid) return `cid:${cid}`;
    const remote = url.searchParams.get('url');
    if (remote && /^(https?):\/\//i.test(remote)) return remote;
  } catch {
    // Leave malformed legacy URLs untouched.
  }
  return src;
}

export function restoreForwardedImageSources(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.querySelectorAll('img[src]').forEach(img => {
    const src = img.getAttribute('src');
    if (src) img.setAttribute('src', portableImageSource(src));
  });
  return doc.body.innerHTML;
}

export function cidReferencesFromHtml(html: string): string[] {
  const found = new Set<string>();
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.querySelectorAll('img[src]').forEach(img => {
    const src = portableImageSource(img.getAttribute('src') ?? '').trim();
    if (src.toLowerCase().startsWith('cid:')) found.add(src.slice(4));
  });
  return [...found];
}
