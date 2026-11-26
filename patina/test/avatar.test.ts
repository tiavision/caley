// <pt-avatar> renders temper's avatar with patina.seed's seed and patina.initials's initials (the component writes
// the two helpers out in its lets: a library's components cannot call the library's helpers).
// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render } from '@tiaforge-ui/sinter/testing';
import { describe, expect, test } from 'vitest';
import * as patina from '../sinter/sinter.helpers.js';

const avatar = readFileSync(join(import.meta.dirname, '..', 'sinter/components/pt-avatar.html'), 'utf8');
const NAMES = ['Ada Lovelace', 'grace hopper', '  Alan  Mathison Turing ', 'Ørjan Ås', '张伟', '(Bob) Chen', 'X', '', '—', 'Émile Zola', '🙂 Smile'];

describe('helpers', () => {
  test('seed is stable, spread and case-blind', () => {
    expect(patina.seed('Ada Lovelace')).toBe(patina.seed('  ada lovelace '));
    const seen = new Set(Array.from({ length: 200 }, (_, i) => patina.seed(`Person ${i}`)));
    expect(seen.size).toBe(12);
    for (const s of seen) expect(Number(s)).toBeGreaterThanOrEqual(0);
  });
  test('initials', () => {
    expect(patina.initials('Ada Lovelace')).toBe('AL');
    expect(patina.initials('  Alan  Mathison Turing ')).toBe('AT');
    expect(patina.initials('(Bob) Chen')).toBe('BC');
    expect(patina.initials('张伟')).toBe('张');
    expect(patina.initials('')).toBe('?');
    expect(patina.initials('—')).toBe('?');
  });
});

describe('<pt-avatar>', () => {
  test('matches the helpers for every name', async () => {
    const page = `<div data-sn-state="{ names: ${JSON.stringify(NAMES).replace(/"/g, "'")} }">
      <template data-sn-for="n in names"><pt-avatar data-sn-with="{ name: n }"></pt-avatar></template></div>`;
    const r = await render(page, { components: { 'pt-avatar': avatar }, types: 'export type AvatarSize = "sm" | "md" | "lg" | "xl";' } as never);
    await r.settle();
    const spans = [...r.root.querySelectorAll('[data-avatar]')];
    expect(spans.map((s) => s.getAttribute('data-pt-seed'))).toEqual(NAMES.map(patina.seed));
    expect(spans.map((s) => s.textContent)).toEqual(NAMES.map(patina.initials));
    expect(spans.every((s) => s.getAttribute('aria-hidden') === 'true')).toBe(true);
  });
  test('a photo, a size, a halo', async () => {
    const r = await render(`<pt-avatar data-sn-with="{ name: 'Ada', src: '/a.jpg', size: 'lg', halo: true }"></pt-avatar>
      <pt-avatar data-sn-with="{ name: 'Ada', size: 'xl' }"></pt-avatar>`, { components: { 'pt-avatar': avatar }, types: 'export type AvatarSize = "sm" | "md" | "lg" | "xl";' } as never);
    await r.settle();
    const [a, b] = r.root.querySelectorAll<HTMLElement>('[data-avatar]');
    expect(a.querySelector('img')?.getAttribute('src')).toBe('/a.jpg');
    expect(a.getAttribute('data-size')).toBe('lg');
    expect(a.hasAttribute('data-pt-halo')).toBe(true);
    expect(b.hasAttribute('data-size')).toBe(false);
    expect(b.style.getPropertyValue('--tm-avatar-size')).toBe('4.5rem');
    expect(b.textContent).toBe('A');
  });
});
