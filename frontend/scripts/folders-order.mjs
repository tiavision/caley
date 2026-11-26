// A browser check of the mail sidebar's folder order (alloy's <alloy-sortable>) against the build a DEMO_MODE
// backend serves (`npm run build`, then the backend on :3000; `npm run check:folders`; CALEY_URL for another origin).
// It signs in through the demo's POST /auth/login, creates three virtual folders, and in Chromium:
//   - drags one by its handle with the mouse: the row follows the pointer, a line shows where it drops, the order
//     changes at once and is still so after a reload (saved);
//   - moves one with the keyboard (Space, ArrowDown, Space): announced in the live region, the handle keeps the focus;
//   - a second tab shows the new order without a reload (folders_changed);
//   - a touch drag (a phone's viewport, the sidebar as a drawer) moves one too;
// and fails on a console error, a page error or a CSP violation. The folders are deleted afterwards.
// SHOTS=<dir> writes screenshots of the drag and of the result there.
import { mkdirSync } from 'node:fs';
import { createRequire } from 'node:module';

// Playwright as installed (the project's, or a global one on NODE_PATH: require finds both, an ESM import only the first)
const { chromium, devices } = createRequire(import.meta.url)('playwright');

const base = process.env.CALEY_URL ?? 'http://localhost:3000';
const shots = process.env.SHOTS;
if (shots) mkdirSync(shots, { recursive: true });
const NAMES = ['e2e-Alpha', 'e2e-Beta', 'e2e-Gamma'];
let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
const login = await context.request.post(`${base}/auth/login`);
if (!login.ok()) throw new Error(`demo sign-in failed (${login.status()}): is the backend in DEMO_MODE?`);

const api = context.request;
const before = (await (await api.get(`${base}/api/email/folders`)).json()).folders;
for (const f of before.filter((f) => f.display_name.startsWith('e2e-'))) await api.delete(`${base}/api/email/folder?folder_id=${f.folder_id}`);
const ids = [];
for (const display_name of NAMES) ids.push((await (await api.post(`${base}/api/email/folder`, { data: { display_name, rules: [] } })).json()).folder_id);
// these three first, in this order (other folders the demo user has follow them)
await api.put(`${base}/api/email/folders/order`, { data: { folder_ids: ids } });
const total = (await (await api.get(`${base}/api/email/folders`)).json()).folders.length;

/** a tab that records console errors, page errors and CSP violations */
async function tab(ctx = context) {
  const page = await ctx.newPage();
  const errors = [];
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
  page.on('pageerror', (e) => errors.push(e.message));
  await page.addInitScript(() => {
    window.__csp = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__csp.push(`${e.violatedDirective} ${e.blockedURI}`));
  });
  await page.goto(`${base}/en/mail/`);
  await page.waitForSelector('.alloy-sortable li[data-al-key]');
  return { page, errors };
}
const order = (page) => page.$$eval('.alloy-sortable li[data-al-key] .sidebar-item [data-truncate]', (els) => els.map((e) => e.textContent.trim()).filter((t) => t.startsWith('e2e-')));
const handle = (page, name) => page.locator(`.alloy-sortable button.al-handle[aria-label="Move ${name}"]`);
const waitOrder = async (page, want, ms = 5000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (JSON.stringify(await order(page)) === JSON.stringify(want)) return true; await page.waitForTimeout(50); }
  return false;
};

const a = await tab();
const b = await tab();
check(JSON.stringify(await order(a.page)) === JSON.stringify(NAMES), `the folders in their order: ${await order(a.page)}`);

// ── mouse drag: Gamma by its handle to the top ──
const row = a.page.locator('.alloy-sortable li[data-al-key]').filter({ hasText: 'e2e-Gamma' });
await row.hover();
check(await handle(a.page, 'e2e-Gamma').evaluate((el) => getComputedStyle(el).opacity === '1'), 'a quiet handle shows on hover');
const hb = await handle(a.page, 'e2e-Gamma').boundingBox();
const top = await a.page.locator('.alloy-sortable li[data-al-key]').filter({ hasText: 'e2e-Alpha' }).boundingBox();
await a.page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
await a.page.mouse.down();
for (let i = 1; i <= 10; i++) await a.page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2 + (top.y + 4 - hb.y - hb.height / 2) * i / 10);
await a.page.waitForTimeout(50);
check(await a.page.locator('li[data-al-lifted="pointer"]').count() === 1, 'the dragged row is lifted');
const dy = await a.page.locator('li[data-al-lifted="pointer"]').evaluate((li) => new DOMMatrix(getComputedStyle(li).transform).m42);
check(dy < -40, `it follows the pointer (translateY ${Math.round(dy)}px)`);
check(await a.page.locator('li[data-al-line="before"]').filter({ hasText: 'e2e-Alpha' }).count() === 1, 'the drop line is above Alpha');
if (shots) await a.page.locator('.mail-sidebar').screenshot({ path: `${shots}/drag.png` });
await a.page.mouse.up();
check(await waitOrder(a.page, ['e2e-Gamma', 'e2e-Alpha', 'e2e-Beta'], 300), 'dropped: the order changes at once');
check(await a.page.locator('li[data-al-lifted], li[data-al-line]').count() === 0, 'nothing lifted or lined after the drop');
check(await waitOrder(b.page, ['e2e-Gamma', 'e2e-Alpha', 'e2e-Beta']), 'the other tab follows (folders_changed)');

// ── keyboard: Alpha one down ──
await handle(a.page, 'e2e-Alpha').focus();
await a.page.keyboard.press('Space');
const status = () => a.page.locator('.alloy-sortable [role="status"]').textContent();
check((await status()) === `Picked up e2e-Alpha. Position 2 of ${total}.`, `announced: ${await status()}`);
await a.page.keyboard.press('ArrowDown');
// below Beta: after it when it is the last row, else before the row that follows it
const line = await a.page.$$eval('.alloy-sortable li[data-al-key]', (rows) => rows.map((r) => [r.textContent.includes('e2e-Beta'), r.getAttribute('data-al-line')]));
const beta = line.findIndex(([b]) => b);
check(line[beta][1] === 'after' || line[beta + 1]?.[1] === 'before', 'the drop line is below Beta');
await a.page.keyboard.press('Space');
check(await waitOrder(a.page, ['e2e-Gamma', 'e2e-Beta', 'e2e-Alpha'], 300), 'the keyboard move changes the order at once');
check((await status()) === `Dropped e2e-Alpha at position 3 of ${total}.`, `announced: ${await status()}`);
await a.page.waitForTimeout(100);
check(await a.page.evaluate(() => document.activeElement?.getAttribute('aria-label')) === 'Move e2e-Alpha', 'the moved row\'s handle keeps the focus');
// Escape puts it back
await a.page.keyboard.press('Space');
await a.page.keyboard.press('ArrowUp');
await a.page.keyboard.press('Escape');
check(await waitOrder(a.page, ['e2e-Gamma', 'e2e-Beta', 'e2e-Alpha'], 300), 'Escape cancels a keyboard move');

// ── saved: a reload shows the same order ──
await a.page.waitForTimeout(300);
await a.page.reload();
await a.page.waitForSelector('.alloy-sortable li[data-al-key]');
check(await waitOrder(a.page, ['e2e-Gamma', 'e2e-Beta', 'e2e-Alpha']), 'after a reload: the saved order');
if (shots) await a.page.locator('.mail-sidebar').screenshot({ path: `${shots}/after.png` });

// ── touch, on a phone: the sidebar is a drawer ──
const phone = await browser.newContext({ ...devices['Pixel 7'], storageState: await context.storageState() });
const p = await tab(phone);
// the folders button beside the list's title opens the sidebar as a drawer (the backdrop has the same label, hidden)
await p.page.locator('button[aria-label="Folders"]:visible').first().click();
await p.page.waitForTimeout(300);
const th = await handle(p.page, 'e2e-Alpha').boundingBox();
const tt = await p.page.locator('.alloy-sortable li[data-al-key]').filter({ hasText: 'e2e-Gamma' }).boundingBox();
check(!!th && !!tt, 'on a phone the handles are there (always shown on touch)');
if (th && tt) {
  const cdp = await phone.newCDPSession(p.page);
  const at = (y) => [{ x: th.x + th.width / 2, y }];
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: at(th.y + th.height / 2) });
  for (let i = 1; i <= 10; i++) await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: at(th.y + th.height / 2 + (tt.y + 3 - th.y - th.height / 2) * i / 10) });
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  check(await waitOrder(p.page, ['e2e-Alpha', 'e2e-Gamma', 'e2e-Beta'], 1000), `a touch drag moves it: ${await order(p.page)}`);
  if (shots) await p.page.screenshot({ path: `${shots}/phone.png` });
}

for (const [name, t] of [['desktop', a], ['second tab', b], ['phone', p]]) {
  check(t.errors.length === 0, `${name}: no console or page errors ${t.errors.join(' | ')}`);
  const csp = await t.page.evaluate(() => window.__csp);
  check(csp.length === 0, `${name}: no CSP violation ${csp.join(' | ')}`);
}

for (const id of ids) await api.delete(`${base}/api/email/folder?folder_id=${id}`);
await browser.close();
console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
process.exit(failed ? 1 : 0);
