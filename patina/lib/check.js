// patina check: patina's vocabulary (vocab.json) checked by temper's own checker.
//
// temper's checker takes a vocabulary as `ctx.vocab`, so patina does not parse markup itself: it runs temper's
// checkMarkup with patina's vocab.json and keeps the findings about patina's attributes (bad values, booleans,
// a missing companion attribute, the wrong element or parent), renumbered TM1xx → PT1xx; and since every patina
// attribute is in the data-pt-* namespace, any data-pt-* name the vocabulary does not list is an error (PT104).
// temper's other rules (labels, structure) are temper check's job and are dropped here, so the two checkers never
// report one thing twice. CSS: --pt-* tokens must be patina's, and the derived ones are not set by the app.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { extname, join } from 'node:path';
import { pathToFileURL } from 'node:url';

// temper is a peer: the app's own temper (resolved from the working directory, the app's root) checks the app, the
// same version its temper check runs; patina's own copy (a devDependency) when it is run in its own folder. A
// package linked from elsewhere (a file: dependency) resolves from its real folder, which would miss the app's.
const fromApp = createRequire(join(process.cwd(), 'package.json'));
const fromHere = createRequire(import.meta.url);
const resolveTemper = (id) => { try { return fromApp.resolve(id); } catch { return fromHere.resolve(id); } };
const { checkMarkup, walk, closest } = await import(pathToFileURL(resolveTemper('@tiaforge-ui/temper/check')).href);
const TEMPER = JSON.parse(readFileSync(resolveTemper('@tiaforge-ui/temper/vocab.json'), 'utf8'));
export const VOCAB = JSON.parse(readFileSync(new URL('../vocab.json', import.meta.url), 'utf8'));

const NAMES = Object.keys(VOCAB.attributes);
const TOKENS = Object.keys(VOCAB.tokens);
const MARKUP = new Set(['.html', '.htm', '.svelte', '.astro', '.vue', '.jsx', '.tsx']);
const STYLES = new Set(['.css', '.scss', '.sass', '.less']);
/** temper's codes that come from the vocabulary; the others are temper's own rules */
const VOCAB_CODES = new Set(['TM101', 'TM102', 'TM104', 'TM107', 'TM108', 'TM109', 'TM111', 'TM112']);
const mentions = (message) => NAMES.some((n) => new RegExp(`(^|[^\\w-])${n}(?![\\w-])`).test(message));

/** Checks markup text: patina attributes only. */
export function checkMarkupText(text, file = '<input>') {
  const out = [];
  for (const d of checkMarkup(text, file, { vocab: VOCAB, modules: null, lint: false })) {
    // unknown names are found below: every data-pt-* name is patina's, so it is known or an error, not a guess
    if (!VOCAB_CODES.has(d.code) || d.code === 'TM104') continue;
    if (!mentions(d.message)) continue;
    out.push({ ...d, code: d.code.replace('TM', 'PT') });
  }
  // data-pt-* is patina's namespace: a name in it that vocab.json does not list is an error (also as a key of
  // sinter's data-sn-attr). Comments are blanked first, keeping positions.
  const t = text.replace(/<!--[^]*?-->/g, (c) => c.replace(/[^\n]/g, ' '));
  for (const m of t.matchAll(/(?<![\w-])(data-pt-[a-z0-9-]*)/g)) {
    if (m[1] in VOCAB.attributes) continue;
    const g = closest(m[1], NAMES, 3);
    const lines = t.slice(0, m.index).split('\n');
    out.push({ file, line: lines.length, col: lines.at(-1).length + 1, severity: 'error', code: 'PT104',
      message: `${m[1]} is not a patina attribute.${g ? ` Did you mean ${g}?` : ''} (data-pt-* is patina's namespace)` });
  }
  return out.sort((x, y) => x.line - y.line || x.col - y.col);
}

/** Checks CSS text: --pt-* tokens. */
export function checkStylesText(text, file = '<input>') {
  const out = [];
  const at = (i) => { const lines = text.slice(0, i).split('\n'); return { line: lines.length, col: lines.at(-1).length + 1 }; };
  const css = text.replace(/\/\*[^]*?\*\//g, (c) => c.replace(/[^\n]/g, ' '));
  for (const m of css.matchAll(/var\(\s*(--pt-[a-z0-9-]+)/g)) {
    if (TOKENS.includes(m[1])) continue;
    const g = closest(m[1], TOKENS, 4);
    out.push({ file, ...at(m.index), severity: 'error', code: 'PT201', message: `${m[1]} is not a patina token.${g ? ` Did you mean ${g}?` : ''}` });
  }
  for (const m of css.matchAll(/(?<![\w-])(--pt-[a-z0-9-]+)\s*:/g)) {
    const spec = VOCAB.tokens[m[1]];
    if (!spec) out.push({ file, ...at(m.index), severity: 'warning', code: 'PT202', message: `${m[1]} uses patina's reserved --pt- prefix but is not a patina token. Use your own prefix.` });
    else if (spec.kind === 'derived') out.push({ file, ...at(m.index), severity: 'warning', code: 'PT203', message: `${m[1]} is derived by patina from --tm-hue and --tm-chroma. Set those instead.` });
  }
  return out;
}

/** Checks one file from disk. */
export function checkFile(file) {
  const ext = extname(file);
  const text = readFileSync(file, 'utf8');
  if (MARKUP.has(ext)) return checkMarkupText(text, file);
  if (STYLES.has(ext)) return checkStylesText(text, file);
  return [];
}

/** Whether any source under the paths imports patina's stylesheet. */
export function importsPatina(paths) {
  for (const p of paths) for (const f of walk(p, new Set(['.ts', '.js', '.mjs', '.css', '.scss', '.html']))) {
    if (/@tiaforge-ui\/patina\/patina\.css/.test(readFileSync(f, 'utf8'))) return true;
  }
  return false;
}

/**
 * Checks every markup and CSS file under the paths. PT110: patina attributes are used but nothing imports
 * '@tiaforge-ui/patina/patina.css' (set `css: false` to skip, e.g. when checking a single file).
 */
export function check(paths, { css = true } = {}) {
  const out = [];
  let used = null;
  for (const p of paths) for (const f of walk(p)) {
    out.push(...checkFile(f));
    if (!used && MARKUP.has(extname(f)) && NAMES.some((n) => readFileSync(f, 'utf8').includes(n))) used = f;
  }
  if (css && used && !importsPatina(paths))
    out.push({ file: used, line: 1, col: 1, severity: 'error', code: 'PT110', message: "This project uses patina attributes but does not import '@tiaforge-ui/patina/patina.css'." });
  return out;
}

export { closest };
