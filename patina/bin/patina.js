#!/usr/bin/env node
// patina check [paths...]: patina's vocabulary in markup and its tokens in CSS (lib/check.js).
// Exits with 1 when there are errors (or warnings, with --warnings-as-errors), 2 for a usage error.
import { existsSync, readFileSync } from 'node:fs';
import { check } from '../lib/check.js';

const USAGE = `Usage: patina check [--warnings-as-errors] [paths...]

Checks patina attributes (data-surface, data-display, data-bento, …) in HTML/Svelte/Astro/Vue/JSX files and
--pt-* tokens in CSS, against patina's vocab.json. temper's own rules stay with temper check.`;

const argv = process.argv.slice(2);
if (!argv.length || argv[0] === '--help' || argv[0] === '-h') { console.log(USAGE); process.exit(0); }
if (argv[0] === '--version' || argv[0] === '-v') {
  console.log(JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version);
  process.exit(0);
}
if (argv[0] !== 'check') { console.error(`patina: unknown command ${argv[0]}. The command is: patina check [paths...]`); process.exit(2); }
const strict = argv.includes('--warnings-as-errors');
const paths = argv.slice(1).filter((a) => a !== '--warnings-as-errors');
for (const p of paths) if (p.startsWith('-') || !existsSync(p)) { console.error(`patina: ${p.startsWith('-') ? 'unknown option' : 'no such path'} ${p}`); process.exit(2); }

const results = check(paths.length ? paths : ['.']);
for (const r of results) console.log(`${r.file}:${r.line}:${r.col} ${r.severity} ${r.code} ${r.message}`);
const errors = results.filter((r) => r.severity === 'error').length;
const warnings = results.length - errors;
console.log(`patina check: ${errors} error(s), ${warnings} warning(s)`);
process.exit(errors || (strict && warnings) ? 1 : 0);
