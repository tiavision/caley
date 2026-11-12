// Builds the frontend and runs temper-inspect on every page of the build, logged in.
//
//   npm run inspect                         all pages (`vite build`, then the DEMO_MODE backend on :3000 serves dist)
//   npm run inspect -- --sheet /tmp/sheets  extra options go to temper-inspect
//   npm run inspect -- /mail/ /contacts/    only these pages
//
// It logs in through POST /auth/login and passes the session cookie with --cookie. Demo mode accepts any
// credentials; against a real CalDAV login set INSPECT_CALDAV_URL, INSPECT_USER and INSPECT_PASSWORD.
// INSPECT_BASE changes the server (default http://localhost:3000, the backend serving dist; a built page never
// hot-reloads mid-run, unlike the Vite dev server). The login page is inspected
// without the cookie (signed in, it redirects to the app).
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const base = (process.env.INSPECT_BASE ?? 'http://localhost:3000').replace(/\/$/, '');
const APP_PAGES = ['/calendar/', '/contacts/', '/search/', '/settings/', '/mail/'];

const args = process.argv.slice(2);
// temper-inspect's options that take a value (`--sheet /tmp/x`); the others (`--no-overlays`) are switches.
const VALUE_OPTIONS = ['--widths', '--schemes', '--settle', '--timeout', '--limit', '--sheet', '--browser', '--cookie', '--header'];
const paths = args.filter((a, i) => a.startsWith('/') && !VALUE_OPTIONS.includes(args[i - 1]));
const options = args.filter(a => !paths.includes(a));
const pages = paths.length ? paths : ['/', ...APP_PAGES];

async function login() {
  const body = new URLSearchParams({
    caldav_base_url: process.env.INSPECT_CALDAV_URL ?? 'https://demo.example',
    caldav_username: process.env.INSPECT_USER ?? 'demo',
    caldav_password: process.env.INSPECT_PASSWORD ?? 'demo',
  });
  let res;
  try {
    res = await fetch(`${base}/auth/login`, { method: 'POST', body, redirect: 'manual' });
  } catch (e) {
    throw new Error(`No dev server at ${base} (${e.cause?.code ?? e.message}). Start the backend and \`npm run dev\` first.`);
  }
  const id = res.headers.getSetCookie().map(c => c.split(';')[0]).find(c => c.startsWith('id='));
  if (!res.ok || !id) throw new Error(`Login at ${base}/auth/login failed (HTTP ${res.status}). Is the backend in DEMO_MODE, or are INSPECT_* set?`);
  return id;
}

const bin = fileURLToPath(new URL('../node_modules/.bin/temper-inspect', import.meta.url));
const run = (urls, extra = []) => spawnSync(bin, [...urls, ...extra, ...options], { stdio: 'inherit' }).status ?? 1;

try {
  const app = pages.filter(p => p !== '/').map(p => base + p);
  let status = 0;
  if (pages.includes('/')) status = Math.max(status, run([base + '/']));
  if (app.length) status = Math.max(status, run(app, ['--cookie', await login()]));
  process.exit(status);
} catch (e) {
  console.error(e.message);
  process.exit(2);
}
