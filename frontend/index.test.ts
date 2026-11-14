import { expect, test } from 'vitest';
import { render, mockRequests, memoryStorage, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from './src/test-messages.ts';
import page from './index.html?raw';
import { app } from './src/test-components.ts';
import { localeUrl } from './src/lib/locale-path.ts';
import './src/test-dom.ts';                 // jsdom: dialogs, popovers, commandfor

// <auth-return>'s script navigates away: here it only counts that it ran.
let returned = 0;
const scripts = { 'auth-return': class { connected() { returned++; } } };
const options = { ...app, scripts, helpers: { localeUrl } };

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '' };

async function show(routes: Record<string, RouteResponse> = {}, storage?: Storage) {
  const requests = mockRequests({
    'GET /api/user': null,                       // signed out (sinter.config.ts maps the 401 to null here)
    'GET /api/auth/config': { oidc_enabled: false },
    ...routes,
  });
  returned = 0;
  const r = await render(page, { ...options, requests, messages, storage });
  await r.settle();
  return { ...r, requests };
}

function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

function fill(root: HTMLElement, url: string) {
  type(root.querySelector('[name=caldav_base_url]')!, url);
  type(root.querySelector('[name=caldav_username]')!, 'ada');
  type(root.querySelector('[name=caldav_password]')!, 'secret');
}

test('signed out: the form, without single sign-on', async () => {
  const { root } = await show();
  expect(root.querySelector('h1')!.textContent).toBe('Caley');
  expect(root.querySelector('form')).not.toBeNull();
  expect(root.querySelector('[aria-busy]')).toBeNull();
  expect(root.querySelector('a[href="/auth/oidc"]')).toBeNull();
  expect(returned).toBe(0);
});

test('loading until both answers are in', async () => {
  const requests = mockRequests({ 'GET /api/user': null, 'GET /api/auth/config': { oidc_enabled: true } });
  const { root, flush, settle } = await render(page, { ...options, requests, messages });
  await flush();
  expect(root.querySelector('p[aria-busy=true]')!.textContent).toBe('Loading...');
  expect(root.querySelector('form')).toBeNull();
  await settle();
  expect(root.querySelector('p[aria-busy=true]')).toBeNull();
  expect(root.querySelector('a[href="/auth/oidc"]')!.textContent).toBe('Sign in with single sign-on');
});

test('signed in already: straight to the app', async () => {
  const { root } = await show({ 'GET /api/user': user });
  expect(returned).toBe(1);
  expect(root.querySelector('form')).toBeNull();
});

test('signs in with the form fields, then goes to the app', async () => {
  let signedIn = false;
  const { root, settle, requests } = await show({
    'GET /api/user': () => (signedIn ? user : null),
    'POST /auth/login': () => { signedIn = true; return { status: 'ok' }; },
  });
  fill(root, 'https://dav.example');
  root.querySelector('form')!.requestSubmit();
  await settle();
  const post = requests.calls.find((c) => c.method === 'POST')!;
  expect(post.headers['content-type']).toMatch(/application\/x-www-form-urlencoded/);
  expect(Object.fromEntries(new URLSearchParams(String(post.body)))).toEqual({
    caldav_base_url: 'https://dav.example', caldav_username: 'ada', caldav_password: 'secret',
  });
  expect(returned).toBe(1);
});

test('a refused login shows the server reason and enables the button again', async () => {
  const { root, settle } = await show({ 'POST /auth/login': { status: 400, body: { error: 'Connection failed: nope' } } });
  fill(root, 'https://dav.example');
  root.querySelector('form')!.requestSubmit();
  await settle();
  expect(root.querySelector('[role=alert]')!.textContent).toBe('Connection failed: nope');
  const button = root.querySelector<HTMLButtonElement>('button[type=submit]')!;
  expect(button.disabled).toBe(false);
  expect(button.hasAttribute('aria-busy')).toBe(false);
});

test('a network error shows the generic message', async () => {
  const { root, settle } = await show({ 'POST /auth/login': { status: 0 } });
  fill(root, 'https://dav.example');
  root.querySelector('form')!.requestSubmit();
  await settle();
  expect(root.querySelector('[role=alert]')!.textContent).toBe('Login failed');
});

test('remembers the server URL for the next visit', async () => {
  const storage = memoryStorage();
  const first = await show({}, storage);
  type(first.root.querySelector('[name=caldav_base_url]')!, 'https://dav.example');
  await first.flush();
  const second = await show({}, storage);
  expect(second.root.querySelector<HTMLInputElement>('[name=caldav_base_url]')!.value).toBe('https://dav.example');
});
