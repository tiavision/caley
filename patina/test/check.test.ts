// patina check: temper's checker with patina's vocabulary, its findings about patina's attributes only.
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import { check, checkMarkupText, checkStylesText } from '../lib/check.js';

const PKG = join(import.meta.dirname, '..');
const codes = (html: string) => checkMarkupText(html, 'x.html').map((d: { code: string }) => d.code);

describe('markup', () => {
  test('valid patina markup has no findings', () => {
    expect(codes(`<body data-pt-surface="mesh" data-pt-grain data-pt-pairing="editorial">
      <h1 data-pt-display="hero" data-pt-ink="gradient">A</h1>
      <div data-layout="grid" data-pt-bento><article data-pt-span="wide" data-pt-elevation="raised" data-pt-halo>x</article></div>
      <span data-avatar data-pt-seed="11">AB</span><data data-pt-figure value="3">3</data><header data-pt-hero></header></body>`)).toEqual([]);
  });
  test('a value patina does not have', () => {
    const [d] = checkMarkupText('<div data-pt-surface="frosted"></div>', 'x.html');
    expect(d.code).toBe('PT101');
    expect(d.message).toContain('glass, gradient, mesh');
  });
  test('a value on a boolean', () => expect(codes('<div data-pt-grain="yes"></div>')).toEqual(['PT102']));
  test('a data-pt-* name patina does not have', () => {
    const [d] = checkMarkupText('<div data-pt-surfase="glass"></div>', 'x.html');
    expect(d.code).toBe('PT104');
    expect(d.message).toContain('data-pt-surface');
  });
  test('bento needs temper\'s grid', () => expect(codes('<div data-pt-bento></div>')).toEqual(['PT107']));
  test('a span outside a bento', () => expect(codes('<div data-layout="row"><p data-pt-span="wide"></p></div>')).toEqual(['PT109']));
  test('display type on the wrong element', () => expect(codes('<ul data-pt-display="xl"></ul>')).toEqual(['PT108']));
  test('values bound by sinter are checked too', () => expect(codes(`<span data-sn-attr="{ 'data-pt-seed': on ? '12' : '1' }"></span>`)).toEqual(['PT101']));
  test('temper\'s attributes and rules are left to temper check', () => {
    expect(codes('<button data-icon="x"></button><div data-gap="huge" data-layout="row"></div><div data-tone="danger"></div>')).toEqual([]);
  });
});

test('an unknown data-pt-* name, even far from any patina name, also as a data-sn-attr key', () => {
  expect(codes(`<div data-pt-sparkle></div><p data-sn-attr="{ 'data-pt-glow': '' }"></p><!-- data-pt-x in a comment -->`)).toEqual(['PT104', 'PT104']);
});

describe('CSS', () => {
  test('tokens', () => {
    const found = checkStylesText('a { color: var(--pt-ink-3); --pt-glow: red; --pt-blur: 4px; --pt-mine: 1px; }', 'x.css');
    expect(found.map((d: { code: string }) => d.code)).toEqual(['PT201', 'PT203', 'PT202']);
    expect(found[0].message).toContain('--pt-ink-1');
  });
});

describe('the fixtures', () => {
  test('patina check passes on them', () => expect(check([join(PKG, 'fixtures')], { css: false })).toEqual([]));
  test('the CLI exits 0 on them and 1 on a bad page', () => {
    const ok = spawnSync(process.execPath, ['bin/patina.js', 'check', 'fixtures'], { cwd: PKG, encoding: 'utf8' });
    expect(ok.stdout).toContain('0 error(s)');
    const bad = spawnSync(process.execPath, ['bin/patina.js', 'check', 'test/bad'], { cwd: PKG, encoding: 'utf8' });
    expect(bad.status).toBe(1);
    expect(bad.stdout).toMatch(/PT101/);
  });
  test('sinter check --strict compiles and type-checks them', () => {
    const r = spawnSync(process.execPath, [join(PKG, 'node_modules/@tiaforge-ui/sinter/dist/cli.js'), 'check', '--strict'], { cwd: PKG, encoding: 'utf8' });
    const out = r.stdout + r.stderr;
    expect(out.split('\n').filter((l) => / (error|warning) SN\d+/.test(l))).toEqual([]);
    expect(out).toMatch(/0 error\(s\), 0 warning\(s\)/);
    expect(r.status).toBe(0);
  }, 300_000);
});
