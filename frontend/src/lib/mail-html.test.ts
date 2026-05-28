import { describe, expect, it } from 'vitest';
import { imageOriginsFromHtml, prepareEmailHtml } from './mail-html.ts';

function body(html: string): HTMLElement {
  return new DOMParser().parseFromString(html, 'text/html').body;
}

describe('prepareEmailHtml', () => {
  it('rewrites remote images through the proxy and reports origins', () => {
    const html = prepareEmailHtml('<img src="https://img.example.test/pixel.png?x=1&y=2" alt="pixel">');
    const img = body(html).querySelector('img')!;

    expect(img.getAttribute('src')).toBe('/api/email/proxy-image?url=https%3A%2F%2Fimg.example.test%2Fpixel.png%3Fx%3D1%26y%3D2');
    expect(imageOriginsFromHtml(html)).toEqual(['https://img.example.test']);
  });

  it('rewrites cid images only when message context is available', () => {
    const html = prepareEmailHtml('<img src="cid:logo@example.test">', {
      accountId: 'acct 1',
      mailbox: 'INBOX/Receipts',
      uid: 42,
    });
    const img = body(html).querySelector('img')!;

    expect(img.getAttribute('src')).toBe('/api/email/proxy-image?account_id=acct%201&mailbox=INBOX%2FReceipts&uid=42&cid=logo%40example.test');

    const withoutContext = prepareEmailHtml('<img src="cid:logo@example.test">');
    expect(body(withoutContext).querySelector('img')!.hasAttribute('src')).toBe(false);
  });

  it('allows non-svg data images only on img src', () => {
    const html = prepareEmailHtml(`
      <img src="data:image/png;base64,iVBORw0KGgo=">
      <img src="data:image/svg+xml;base64,PHN2Zy8+">
      <a href="data:image/png;base64,iVBORw0KGgo=">data link</a>
    `);
    const doc = body(html);
    const imgs = doc.querySelectorAll('img');

    expect(imgs[0].getAttribute('src')).toBe('data:image/png;base64,iVBORw0KGgo=');
    expect(imgs[1].hasAttribute('src')).toBe(false);
    expect(doc.querySelector('a')!.hasAttribute('href')).toBe(false);
  });

  it('strips direct-load tags and attributes outside img src', () => {
    const html = prepareEmailHtml(`
      <picture><source src="https://tracker.example.test/source.png" srcset="https://tracker.example.test/2x.png 2x"><img srcset="https://tracker.example.test/set.png 2x" src="https://tracker.example.test/img.png"></picture>
      <video poster="https://tracker.example.test/poster.png" src="https://tracker.example.test/movie.mp4"></video>
      <table background="https://tracker.example.test/bg.png"><tr><td>ok</td></tr></table>
    `);
    const doc = body(html);

    expect(doc.querySelector('picture')).toBeNull();
    expect(doc.querySelector('source')).toBeNull();
    expect(doc.querySelector('video')).toBeNull();
    expect(doc.querySelector('img')!.hasAttribute('srcset')).toBe(false);
    expect(doc.querySelector('img')!.getAttribute('src')).toBe('/api/email/proxy-image?url=https%3A%2F%2Ftracker.example.test%2Fimg.png');
    expect(doc.querySelector('table')!.hasAttribute('background')).toBe(false);
  });

  it('keeps safe links and strips unsafe links', () => {
    const html = prepareEmailHtml(`
      <a href="https://example.test/path">https</a>
      <a href="mailto:user@example.test">mail</a>
      <a href="javascript:alert(1)">js</a>
      <a href=" DATA:text/html,<script>alert(1)</script>">data</a>
    `);
    const links = [...body(html).querySelectorAll('a')];

    expect(links[0].getAttribute('href')).toBe('https://example.test/path');
    expect(links[0].getAttribute('target')).toBe('_blank');
    expect(links[0].getAttribute('rel')).toBe('noopener noreferrer');
    expect(links[1].getAttribute('href')).toBe('mailto:user@example.test');
    expect(links[2].hasAttribute('href')).toBe(false);
    expect(links[3].hasAttribute('href')).toBe(false);
  });

  it('sanitizes inline styles', () => {
    const html = prepareEmailHtml('<p style="color: red; background-image: url(https://tracker.example.test/x.png); position: fixed; font-weight: bold">text</p>');
    const style = body(html).querySelector('p')!.getAttribute('style') ?? '';

    expect(style).toContain('color: red');
    expect(style).toContain('font-weight: bold');
    expect(style).not.toContain('url');
    expect(style).not.toContain('position');
  });
});
