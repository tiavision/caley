// Vite plugin: runs patina check in every build (errors fail it) and on the dev server (printed, on start and for
// every changed file), as temper's plugin runs temper check.
//
//   import patina from '@tiaforge-ui/patina/vite';
//   export default { plugins: [temper(), patina({ paths: ['src', 'index.html'] })] };
//
// Options: paths (default: the Vite root), warningsFail (default false).
import { relative, resolve } from 'node:path';
import { check, checkFile } from './check.js';

const CHECKED = /\.(html?|svelte|astro|vue|jsx|tsx|css|scss|sass|less)$/;

export default function patina(options = {}) {
  let root = process.cwd();
  let command = 'build';
  const roots = () => (options.paths ?? ['.']).map((p) => resolve(root, p));
  const print = (logger, results) => {
    for (const r of results) {
      const line = `${relative(root, r.file)}:${r.line}:${r.col} ${r.severity} ${r.code} ${r.message}`;
      if (r.severity === 'error') logger.error(line); else logger.warn(line);
    }
  };
  return {
    name: 'patina',
    configResolved(config) { root = config.root; command = config.command; },
    buildStart() {
      if (command !== 'build') return;
      const results = check(roots());
      const fail = results.filter((r) => r.severity === 'error' || (options.warningsFail && r.severity === 'warning'));
      for (const r of results) {
        const line = `${relative(root, r.file)}:${r.line}:${r.col} ${r.code} ${r.message}`;
        if (fail.includes(r)) continue;
        this.warn(line);
      }
      const errors = results.filter((r) => r.severity === 'error').length;
      console.log(`patina check: ${errors} error(s), ${results.length - errors} warning(s)`);
      if (fail.length) this.error(`patina check: ${fail.length} problem(s)\n${fail.map((r) => `${relative(root, r.file)}:${r.line}:${r.col} ${r.code} ${r.message}`).join('\n')}`);
    },
    configureServer(server) {
      print(server.config.logger, check(roots()));
      server.watcher.on('change', (file) => {
        if (CHECKED.test(file) && roots().some((r) => file.startsWith(r))) print(server.config.logger, checkFile(file));
      });
    },
  };
}
