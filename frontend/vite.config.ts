import { defineConfig } from 'vite';
import { existsSync } from 'fs';
import { resolve } from 'path';
import temper from '@tiaforge-ui/temper/vite';
import sinter from '@tiaforge-ui/sinter/vite';

const root = import.meta.dirname;


export default defineConfig({
  appType: 'mpa',
  plugins: [
    // Every HTML entry is compiled by sinter (partials/, src/sinter.{config,helpers,actions,types}.ts). Messages are
    // sinter's own: messages/{locale}.json of project.inlang (the inlang tooling edits the same files).
    // chunks: false keeps Vite's own chunks instead of sinter's copying of small shared chunks (sinter 0.11). Measured
    // 2026-11-09 on Caley's cold loads over HTTP/2 + TLS + gzip (what Caddy serves in production) at 40 and 300 ms:
    // no difference in first paint or content time (within 8 ms), while copying made login, contacts and settings
    // 8-11 kB gzip bigger (sinter's whole runtime in one shared file). It only paid off over HTTP/1.1.
    sinter({ typecheck: true, chunks: false }),
    // temper check on all markup and CSS (errors fail the build); serves the Lucide icons the markup names.
    // Only existing paths: temper stops the build on a missing one (partials/ arrives with the sinter pages).
    temper({ paths: ['src', 'partials', 'index.html', 'calendar', 'contacts', 'mail', 'search', 'settings'].map((p) => resolve(root, p)).filter(existsSync), icons: { lucide: true }, prune: true }),
  ],
  css: {
    preprocessorOptions: {
      scss: {},
    },
  },
  build: {
    rollupOptions: {
      input: {
        index: resolve(root, 'index.html'),
        calendar: resolve(root, 'calendar/index.html'),
        settings: resolve(root, 'settings/index.html'),
        search: resolve(root, 'search/index.html'),
        contacts: resolve(root, 'contacts/index.html'),
        mail: resolve(root, 'mail/index.html'),
        mailView: resolve(root, 'mail/view/index.html'),
      },
    },
  },
  server: {
    proxy: {
      '/api': { target: 'http://localhost:3000', changeOrigin: true },
      '/auth': 'http://localhost:3000',
      '/logout': 'http://localhost:3000',
    },
  },
});
