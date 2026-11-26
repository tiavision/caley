// Screenshots of the demo pages (README.md here): `node docs/expressive-demo/shots.mjs <label> [base]`, with a caley build served by
// a DEMO_MODE backend (default http://localhost:3000). Writes screenshots/<page>-<label>-<width>-<scheme>.png
// for the login page, the contacts overview and a contact's detail dialog, at desktop and phone width, light and dark.
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');

const [label = 'after', base = 'http://localhost:3000'] = process.argv.slice(2);
const out = join(import.meta.dirname, 'screenshots');
mkdirSync(out, { recursive: true });

const widths = { desktop: { width: 1280, height: 800 }, phone: { width: 390, height: 844 } };
const browser = await chromium.launch();
for (const [w, viewport] of Object.entries(widths)) {
  for (const scheme of ['light', 'dark']) {
    const ctx = await browser.newContext({ viewport, colorScheme: scheme, deviceScaleFactor: 1, reducedMotion: 'reduce' });
    const page = await ctx.newPage();
    const shot = (name) => page.screenshot({ path: join(out, `${name}-${label}-${w}-${scheme}.png`) });

    await page.goto(`${base}/en/`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(300);
    await shot('login');

    // sign in (the demo's button), then the contacts overview
    await page.request.post(`${base}/auth/login`);
    await page.goto(`${base}/en/contacts/`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(400);
    await shot('contacts');

    // the first contact's detail dialog
    await page.locator('main article button[data-tm-stretch]').first().click();
    await page.locator('#contact-detail[open]').waitFor();
    await page.waitForTimeout(400);
    await shot('contact-detail');
    await ctx.close();
  }
}
// a rebrand: the same pages with only --tm-hue changed (expressive.css's colours all follow it), for the "after" build
if (label === 'after') {
  for (const scheme of ['light', 'dark']) {
    const ctx = await browser.newContext({ viewport: widths.desktop, colorScheme: scheme, deviceScaleFactor: 1, reducedMotion: 'reduce' });
    const page = await ctx.newPage();
    const rebrand = () => page.addStyleTag({ content: ':root { --tm-hue: 160; }' });
    await page.goto(`${base}/en/`, { waitUntil: 'networkidle' });
    await rebrand();
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(out, `login-rebrand-hue160-desktop-${scheme}.png`) });
    await page.request.post(`${base}/auth/login`);
    await page.goto(`${base}/en/contacts/`, { waitUntil: 'networkidle' });
    await rebrand();
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(out, `contacts-rebrand-hue160-desktop-${scheme}.png`) });
    await ctx.close();
  }
}
await browser.close();
console.log(`screenshots written to ${out}`);
