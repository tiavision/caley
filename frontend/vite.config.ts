import { defineConfig } from 'vite';
import { paraglideVitePlugin } from '@inlang/paraglide-js';
import { existsSync } from 'fs';
import { resolve } from 'path';
import temper from '@tiaforge-ui/temper/vite';
import sinter from '@tiaforge-ui/sinter/vite';

const root = import.meta.dirname;


export default defineConfig({
  appType: 'mpa',
  plugins: [
    // Every HTML entry is compiled by sinter (partials/, src/sinter.{config,helpers,actions,types}.ts).
    // First paint: sinter writes what it knows at build time into the HTML (English); its first-paint script picks
    // the locale as src/lib/i18n.ts did (localStorage `locale`, else the browser's), sets <html lang> and writes
    // that locale's text and the persisted controls (the rail's collapse checkbox) before the page shows.
    sinter({ typecheck: true, locale: { locales: ['en', 'de'], storage: 'locale', use: 'src/paraglide/runtime.js#setLocale' } }),
    // temper check on all markup and CSS (errors fail the build); serves the Lucide icons the markup names.
    // Only existing paths: temper stops the build on a missing one (partials/ arrives with the sinter pages).
    temper({ paths: ['src', 'partials', 'index.html', 'calendar', 'contacts', 'mail', 'search', 'settings'].map((p) => resolve(root, p)).filter(existsSync), icons: { lucide: true }, prune: true }),
    paraglideVitePlugin({
      project: './project.inlang',
      outdir: './src/paraglide',
    }),
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
