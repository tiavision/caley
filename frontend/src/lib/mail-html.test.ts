import { describe, expect, it } from 'vitest';
import { parse } from '@tiavision/sinter-mail';
import type { Body, Element, Node } from '@tiavision/sinter-mail';
import { cidReferencesFromHtml, withInlineImages } from './mail-html.ts';
import { bodyImages, keepsLight } from './mail.ts';
import { progressive } from './mail-body.ts';

const ref = { account_id: 'acct 1', mailbox: 'INBOX/Receipts', uid: 42 };

function elements(node: Body | Node, tag: string): Element[] {
  if (node.type === 'text') return [];
  const own = 'tag' in node && node.tag === tag ? [node] : [];
  return [...own, ...node.children.flatMap((c) => elements(c, tag))];
}

describe('bodyImages (email.parse options)', () => {
  it('sends remote images through the proxy, versioned, and lists their origins', () => {
    const body = parse('<img src="https://img.example.test/pixel.png?x=1&y=2" alt="pixel">', bodyImages(ref, null, 3));
    expect(elements(body, 'img')[0].attrs.src).toBe('/api/email/proxy-image?v=3&url=https%3A%2F%2Fimg.example.test%2Fpixel.png%3Fx%3D1%26y%3D2');
    expect(body.images.origins).toEqual(['https://img.example.test']);
  });

  it('loads inline images from the message, or from a decrypted message\'s parts', () => {
    const html = '<img src="cid:logo@example.test">';
    expect(elements(parse(html, bodyImages(ref, null, 0)), 'img')[0].attrs.src)
      .toBe('/api/email/proxy-image?account_id=acct%201&mailbox=INBOX%2FReceipts&uid=42&cid=logo%40example.test');
    const data = 'data:image/png;base64,iVBORw0KGgo=';
    expect(elements(parse(html, bodyImages(ref, { 'logo@example.test': data }, 0)), 'img')[0].attrs.src).toBe(data);
  });
});

describe('inline images of a forward', () => {
  it('lists the cid references', () => {
    expect(cidReferencesFromHtml('<img src="cid:logo@example.test"><img src="https://x.test/a.png"><img src=" CID:chart@example.test">'))
      .toEqual(['logo@example.test', 'chart@example.test']);
  });

  it('writes in the decrypted parts, keeping the others', () => {
    expect(withInlineImages('<img src="cid:a.b@x"><img src="cid:c@x">', { 'a.b@x': 'data:image/png;base64,AA==' }))
      .toBe('<img src="data:image/png;base64,AA=="><img src="cid:c@x">');
  });
});

describe('keepsLight', () => {
  it('is true when the mail sets a text or background colour anywhere, by style or legacy attribute', () => {
    expect(keepsLight(parse('<p>plain <b>text</b></p>'))).toBe(false);
    expect(keepsLight(parse('<div><p style="color:#222">dark text, no background</p></div>'))).toBe(true);
    expect(keepsLight(parse('<table><tr><td bgcolor="#f5f5f5">row</td></tr></table>'))).toBe(true);
    expect(keepsLight(parse('<body style="background:#fff"><p>x</p></body>'))).toBe(true);
  });
  it('is false for a mail that says it works on a dark page: its color-scheme, or dark rules', () => {
    expect(keepsLight(parse('<meta name="color-scheme" content="light dark"><p><a href="https://example.com/" style="color:#a85fb0">a</a></p>'))).toBe(false);
    expect(keepsLight(parse('<html style="color-scheme: light dark"><p style="color:dimgray">x</p>'))).toBe(false);
    expect(keepsLight(parse('<style>p { color: #222 } @media (prefers-color-scheme: dark) { p { color: #eee } }</style><p>x</p>'))).toBe(false);
    // light only, or a dark rule that changes no colour: kept light
    expect(keepsLight(parse('<meta name="color-scheme" content="only light"><p style="color:#222">x</p>'))).toBe(true);
    expect(keepsLight(parse('<style>@media (prefers-color-scheme: dark) { img { width: 1px } }</style><p style="color:#222">x</p>'))).toBe(true);
  });
});

describe('mailBody.progressive', () => {
  it('numbers a copy of the frozen tree, the same copy for the same tree', () => {
    const tree = parse('<p>a</p><p>b <b>c</b></p>', {});
    const body = progressive(tree);
    expect(body).not.toBe(tree);
    expect(Object.isFrozen(tree)).toBe(true);
    expect([body.head, body.children.map((c) => c.n)]).toEqual([200, [0, 2]]);
    expect(progressive(tree)).toBe(body);
    expect(progressive(parse('<p>a</p><p>b <b>c</b></p>', {}))).toBe(body);   // parse keeps one tree per source and options
  });
});
