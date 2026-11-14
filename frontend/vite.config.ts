import { defineConfig } from 'vite';
import { existsSync } from 'fs';
import { resolve } from 'path';
import temper from '@tiaforge-ui/temper/vite';
import sinter from '@tiaforge-ui/sinter/vite';

const root = import.meta.dirname;


export default defineConfig({
  appType: 'mpa',
  plugins: [
    // Every HTML entry is compiled by sinter (src/components/, src/layouts/, src/sinter.{config,helpers,actions,types}.ts). Messages are
    // sinter's own: messages/{locale}.json of project.inlang (the inlang tooling edits the same files).
    // fonts.preload: the UI font (Figtree, @fontsource-variable/figtree) is preloaded, so a first visit does not paint the fallback font first.
    sinter({ typecheck: true, embed: true, localePrefix: true, lazy: ['openpgp'], fonts: { preload: ['Figtree Variable'] } }),
    // temper check on all markup and CSS (errors fail the build); serves the Lucide icons the markup names.
    // Only existing paths: temper stops the build on a missing one (components and layouts are under src/).
    // `inspect`: the dev server also audits the rendered page (overflow, contrast, clipped text, opened dialogs) and
    // prints findings in its terminal; agents check the build with `npm run inspect` instead.
    temper({ paths: ['src', 'index.html', 'calendar', 'contacts', 'mail', 'search', 'settings'].map((p) => resolve(root, p)).filter(existsSync), icons: { lucide: true }, prune: true, inspect: true }),
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
