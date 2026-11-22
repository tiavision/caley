import { describe, expect, it } from 'vitest';
import { parse } from '@tiavision/sinter-mail';
import type { Body, Element, Node } from '@tiavision/sinter-mail';
import { cidReferencesFromHtml, withInlineImages } from './mail-html.ts';
import { bodyImages } from './mail.ts';

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
