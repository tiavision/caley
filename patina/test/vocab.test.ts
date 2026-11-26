// vocab.json is the vocabulary: every attribute and value it lists has CSS, every attribute the CSS styles is in it,
// every token agrees, and the CSS keeps the ecosystem's grain (layers, :where(), no !important, colour from temper).
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';

const require = createRequire(import.meta.url);
const PKG = join(import.meta.dirname, '..');
const vocab = JSON.parse(readFileSync(join(PKG, 'vocab.json'), 'utf8'));
const temperVocab = require('@tiaforge-ui/temper/vocab.json');
const css = readFileSync(join(PKG, 'sinter/patina.css'), 'utf8').replace(/\/\*[^]*?\*\//g, '');

describe('vocabulary and CSS agree', () => {
  test('every attribute and value in vocab.json has a rule', () => {
    const missing: string[] = [];
    for (const [name, spec] of Object.entries<any>(vocab.attributes)) {
      if (spec.boolean) { if (!css.includes(`[${name}]`)) missing.push(name); continue; }
      for (const v of Object.keys(spec.values)) if (!css.includes(`[${name}="${v}"]`)) missing.push(`${name}="${v}"`);
    }
    expect(missing).toEqual([]);
  });

  test('every patina attribute the CSS styles is in vocab.json, and no temper attribute is restyled by value', () => {
    const used = new Set([...css.matchAll(/\[(data-[\w-]+)(?:="([^"]*)")?\]/g)].map((m) => m[1]));
    const temper = new Set(Object.keys(temperVocab.attributes));
    const unknown = [...used].filter((n) => !(n in vocab.attributes) && !temper.has(n));
    expect(unknown).toEqual([]);
    // patina only reads temper's attributes as context (data-avatar, data-stretch; data-pad, which replaces the hero's own padding)
    expect([...used].filter((n) => temper.has(n)).sort()).toEqual(['data-avatar', 'data-pad', 'data-stretch']);
  });

  test('tokens: defined in the CSS exactly as vocab.json lists them', () => {
    const defined = new Set([...css.matchAll(/(--pt-[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
    expect([...defined].sort()).toEqual(Object.keys(vocab.tokens).sort());
  });

  test('every rule is in a pt layer and every selector in :where()', () => {
    // top level: only @layer blocks and statements
    let depth = 0;
    let top = '';
    for (const ch of css) {
      if (ch === '{') depth++;
      if (depth === 0) top += ch;
      if (ch === '}') depth--;
    }
    expect(top.replace(/@layer[^;{]*;?/g, '').replace(/@layer[^{]*/g, '').trim()).toBe('');
    for (const m of css.matchAll(/@layer\s+([\w.]+)\s*\{/g)) expect(m[1]).toMatch(/^pt\./);
    const selectors = [...css.matchAll(/(?:^|[;{}])\s*([^;{}@]+?)\s*\{/g)].map((m) => m[1].trim()).filter((s) => !s.startsWith('@'));
    for (const s of selectors) for (const part of s.split(/,(?![^(]*\))/)) expect(part.trim(), s).toMatch(/^:where\(/);
    expect(css).not.toContain('!important');
  });

  test('colour comes from temper: no hex, rgb() or hsl() colours, every oklch() reads --tm-hue (or is neutral)', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b(?![^(]*%3E)/i);
    expect(css).not.toMatch(/\b(rgb|rgba|hsl|hsla)\(/);
    for (const m of css.matchAll(/oklch\(([^()]*(?:\([^()]*(?:\([^()]*\))*[^()]*\))*[^()]*)\)/g)) {
      const body = m[1];
      if (/^\s*[\d.]+\s+0\s+0\b/.test(body)) continue; // white or black, alpha only
      expect(body, `oklch(${body})`).toMatch(/var\(--_pt-(h|sh)\)|var\(--_pt-h\)/);
    }
  });
});

describe('names stay clear of temper', () => {
  test('no patina name or value is one temper check would report', async () => {
    const { checkMarkup } = await import(require.resolve('@tiaforge-ui/temper/check'));
    const attrs = Object.entries<any>(vocab.attributes).map(([n, s]) => (s.boolean ? n : `${n}="${Object.keys(s.values)[0]}"`));
    const markup = `<div data-layout="grid" ${attrs.join(' ')}></div>`;
    const found = checkMarkup(markup, 'x.html', {}).filter((d: any) => /TM10[14]/.test(d.code));
    expect(found).toEqual([]);
    for (const n of Object.keys(vocab.attributes)) expect(temperVocab.attributes[n], n).toBeUndefined();
  });
});
