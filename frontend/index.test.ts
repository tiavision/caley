import { expect, test } from 'vitest';
import { render, mockRequests, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from './src/test-messages.ts';
import page from './index.html?raw';
import { app } from './src/test-components.ts';
import { localeUrl, pagePath } from './src/lib/locale-path.ts';
import './src/test-dom.ts';                 // jsdom: dialogs, popovers, commandfor

const options = { ...app, helpers: { localeUrl, pagePath } };

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '' };

async function show(routes: Record<string, RouteResponse> = {}) {
  const requests = mockRequests({
    'GET /api/user': null,                       // signed out (sinter.config.ts maps the 401 to null here)
    'GET /api/auth/config': { oidc_enabled: true, demo_mode: false },
    ...routes,
  });
  const r = await render(page, { ...options, requests, messages });
  await r.settle();
  return { ...r, requests };
}

const demo = { 'GET /api/auth/config': { oidc_enabled: false, demo_mode: true } };

test('signed out: single sign-on, no credentials form', async () => {
  const { root, navigations } = await show();
  expect(root.querySelector('h1')!.textContent).toBe('Caley');
  expect(root.querySelector('a[href="/auth/oidc"]')!.textContent).toBe('Sign in with single sign-on');
  expect(root.querySelector('form')).toBeNull();
  expect(root.querySelector('input')).toBeNull();
  expect(root.querySelector('[aria-busy]')).toBeNull();
  expect(navigations).toEqual([]);
});

test('loading until both answers are in', async () => {
  const requests = mockRequests({ 'GET /api/user': null, 'GET /api/auth/config': { oidc_enabled: true, demo_mode: false } });
  const { root, flush, settle } = await render(page, { ...options, requests, messages });
  await flush();
  expect(root.querySelector('p[aria-busy=true]')!.textContent).toBe('Loading...');
  expect(root.querySelector('a[href="/auth/oidc"]')).toBeNull();
  await settle();
  expect(root.querySelector('p[aria-busy=true]')).toBeNull();
  expect(root.querySelector('a[href="/auth/oidc"]')!.textContent).toBe('Sign in with single sign-on');
});

test('signed in already: straight to the app', async () => {
  const { root, navigations } = await show({ 'GET /api/user': user });
  expect(navigations).toEqual(['replace http://localhost/calendar/']);
  expect(root.querySelector('a[href="/auth/oidc"]')).toBeNull();
});

test('the demo: one button, no fields, no single sign-on', async () => {
  const { root } = await show(demo);
  expect(root.querySelector('button[type=submit]')!.textContent).toBe('Enter the demo');
  expect(root.querySelector('input')).toBeNull();
  expect(root.querySelector('a[href="/auth/oidc"]')).toBeNull();
});

test('entering the demo posts no credentials, then goes to the page the sign-in answered with', async () => {
  const { root, settle, requests, navigations } = await show({
    ...demo,
    'POST /auth/login': { status: 'ok', redirect: '/en/mail/' },
  });
  root.querySelector('form')!.requestSubmit();
  await settle();
  const post = requests.calls.find((c) => c.method === 'POST')!;
  expect(post.url).toMatch(/\/auth\/login$/);
  expect(String(post.body ?? '')).toBe('');
  expect(navigations).toEqual(['replace http://localhost/en/mail/']);
});

test('a refused demo sign-in shows the server reason and enables the button again', async () => {
  const { root, settle } = await show({ ...demo, 'POST /auth/login': { status: 500, body: { error: 'Session error' } } });
  root.querySelector('form')!.requestSubmit();
  await settle();
  expect(root.querySelector('[role=alert]')!.textContent).toBe('Session error');
  const button = root.querySelector<HTMLButtonElement>('button[type=submit]')!;
  expect(button.disabled).toBe(false);
  expect(button.hasAttribute('aria-busy')).toBe(false);
});

test('a network error shows the generic message', async () => {
  const { root, settle } = await show({ ...demo, 'POST /auth/login': { status: 0 } });
  root.querySelector('form')!.requestSubmit();
  await settle();
  expect(root.querySelector('[role=alert]')!.textContent).toBe('Login failed');
});
