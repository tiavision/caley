// Text on patina keeps WCAG AA (4.5:1) in light and dark mode for any brand: every hue (every 15°) at a low, the
// default and a high --tm-chroma. Chromium resolves the tokens (light-dark(), calc() in oklch()) and paints each
// colour on a canvas, so the check uses the sRGB pixels the browser really draws (out-of-gamut colours included).
// Translucent colours (glass) are painted over each backdrop they are meant for. Also: what the preferences turn off.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { afterAll, beforeAll, describe, expect, test } from 'vitest';

const require = createRequire(import.meta.url);
let playwright: typeof import('playwright') | null = null;
try { playwright = require('playwright'); } catch { /* no browser on this machine: skipped */ }

const PKG = join(import.meta.dirname, '..');
const temperCss = readFileSync(require.resolve('@tiaforge-ui/temper/core.css'), 'utf8');
const patinaCss = readFileSync(join(PKG, 'sinter/patina.css'), 'utf8');

const HUES = Array.from({ length: 24 }, (_, i) => i * 15);
const CHROMAS = [0.04, 0.12, 0.19, 0.22];

/** Runs in the page: for each pair [text, backdrops…], the contrast of text over the backdrops painted in order. */
function measure(input: { pairs: Array<[string, string[]]>; seeds: number }) {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const g = canvas.getContext('2d', { willReadFrequently: true })!;
  const probe = document.createElement('div');
  document.body.append(probe);
  const resolve = (token: string, el: HTMLElement = probe) => { el.style.color = `var(${token})`; return getComputedStyle(el).color; };
  const paint = (colors: string[]) => {
    g.globalCompositeOperation = 'copy';
    g.fillStyle = '#000'; g.fillRect(0, 0, 1, 1);
    g.globalCompositeOperation = 'source-over';
    for (const c of colors) { g.fillStyle = c; g.fillRect(0, 0, 1, 1); }
    return [...g.getImageData(0, 0, 1, 1).data.slice(0, 3)];
  };
  const lum = ([r, gr, b]: number[]) => {
    const f = (v: number) => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
    return 0.2126 * f(r) + 0.7152 * f(gr) + 0.0722 * f(b);
  };
  const ratio = (a: number[], b: number[]) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
  const out: Array<{ pair: string; ratio: number }> = [];
  for (const [text, backdrops] of input.pairs) {
    const bg = paint(backdrops.map((t) => resolve(t)));
    const fg = paint([...backdrops.map((t) => resolve(t)), resolve(text)]);
    out.push({ pair: `${text} on ${backdrops.join(' + ')}`, ratio: ratio(fg, bg) });
  }
  for (let s = 0; s < input.seeds; s++) {
    const el = document.createElement('span');
    el.dataset.ptSeed = String(s);
    document.body.append(el);
    const ink = paint([resolve('--_pt-seed-ink', el)]);
    for (const stop of ['--_pt-seed-a', '--_pt-seed-b']) out.push({ pair: `seed ${s} ink on ${stop}`, ratio: ratio(ink, paint([resolve(stop, el)])) });
    el.remove();
  }
  return out;
}

// what text sits on: temper's page and surface, patina's tints (gradient and mesh are made of them and --tm-bg), glass
const SURFACES = [['--tm-bg'], ['--tm-surface'], ['--pt-tint-1'], ['--pt-tint-2'], ['--pt-tint-3'],
  ['--tm-bg', '--pt-glass'], ['--pt-tint-1', '--pt-glass'], ['--pt-tint-2', '--pt-glass'], ['--pt-tint-3', '--pt-glass']];
const PAIRS: Array<[string, string[]]> = ['--tm-text', '--tm-text-muted', '--tm-accent', '--pt-ink-1', '--pt-ink-2']
  .flatMap((t) => SURFACES.map((s): [string, string[]] => [t, s]));

describe.skipIf(!playwright)('contrast', () => {
  let browser: import('playwright').Browser;
  beforeAll(async () => { browser = await playwright!.chromium.launch(); });
  afterAll(async () => { await browser?.close(); });

  for (const scheme of ['light', 'dark'] as const) {
    test(`AA for every hue and chroma, ${scheme}`, async () => {
      const page = await browser.newPage({ colorScheme: scheme });
      await page.setContent(`<!doctype html><style>${temperCss}</style><style>${patinaCss}</style><body></body>`);
      const failures: string[] = [];
      let worst = Infinity;
      for (const hue of HUES) for (const chroma of CHROMAS) {
        await page.evaluate(([h, c]) => { document.documentElement.style.setProperty('--tm-hue', String(h)); document.documentElement.style.setProperty('--tm-chroma', String(c)); }, [hue, chroma]);
        for (const r of await page.evaluate(measure, { pairs: PAIRS, seeds: 12 })) {
          worst = Math.min(worst, r.ratio);
          if (r.ratio < 4.5) failures.push(`hue ${hue} chroma ${chroma}: ${r.pair} ${r.ratio.toFixed(2)}`);
        }
      }
      await page.close();
      expect(failures).toEqual([]);
      expect(worst).toBeGreaterThanOrEqual(4.5);
    });
  }

  test('prefers-contrast: more and reduced transparency fall back to plain surfaces', async () => {
    const page = await browser.newPage();
    await page.setContent(`<!doctype html><style>${temperCss}</style><style>${patinaCss}</style>
      <body><div id="mesh" data-pt-surface="mesh" data-pt-grain></div><div id="glass" data-pt-surface="glass"></div><h1 id="ink" data-pt-display="hero" data-pt-ink="gradient">A</h1></body>`);
    const read = () => page.evaluate(() => {
      const cs = (id: string) => getComputedStyle(document.getElementById(id)!);
      return { mesh: cs('mesh').backgroundImage, glass: cs('glass').backdropFilter, glassBg: cs('glass').backgroundColor, ink: cs('ink').webkitTextFillColor, inkBg: cs('ink').backgroundImage };
    });
    const normal = await read();
    expect(normal.mesh).toContain('radial-gradient');
    expect(normal.mesh).toContain('url(');
    expect(normal.glass).toContain('blur');
    expect(normal.inkBg).toContain('linear-gradient');

    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-contrast', value: 'more' }] });
    const more = await read();
    expect(more.mesh).toBe('none');
    expect(more.glass).toBe('none');
    expect(more.inkBg).toBe('none');
    expect(more.ink).not.toMatch(/rgba\(0, 0, 0, 0\)|transparent/);

    await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-transparency', value: 'reduce' }] });
    const opaque = await read();
    expect(opaque.glass).toBe('none');
    expect(opaque.glassBg).not.toMatch(/\/ 0\.|rgba/);
    expect(opaque.mesh).toContain('radial-gradient');
    await page.close();
  });
});
