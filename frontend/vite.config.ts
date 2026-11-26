import { defineConfig } from 'vite';
import { existsSync } from 'fs';
import { resolve } from 'path';
import temper from '@tiaforge-ui/temper/vite';
import patina from '@tiaforge-ui/patina/vite';
import sinter from '@tiaforge-ui/sinter/vite';
import { apis } from './apis.ts';

const root = import.meta.dirname;


export default defineConfig({
  appType: 'mpa',
  plugins: [
    // Every HTML entry is compiled by sinter (src/components/, src/layouts/, src/sinter.{config,helpers,actions,types,content}.ts). The
    // languages and messages come from the content module (src/sinter.content.ts): messages/{locale}.json of project.inlang.
    // fonts.preload: the UI font (Figtree, @fontsource-variable/figtree) is preloaded, so a first visit does not paint the fallback font first;
    // so is the display face of patina's editorial pairing (Fraunces), on the pages that import it (login, contacts).
    // apis: the backend's JSON API (apis.ts, described by openapi.json).
    // libraries: patina's <pt-avatar> (a seeded gradient avatar, ../patina), and sinter-mail's <mail-body> (a mail body parsed by its email.parse, src/sinter.helpers.ts) and <mail-width>.
    // lazy: openpgp, and sinter-mail: its parser, its MIME reader and its component scripts (<mail-body>'s builder,
    // <mail-width>, <mail-warm>) load on first use, off the mail pages' bundle; <mail-warm> starts that while the page
    // waits for the message. The actions await every mail.* call.
    // compileHints: V8 compiles each page module as it loads (the mail pages parse a mail in the browser at once).
    sinter({
      typecheck: true, strict: true, embed: true, localePrefix: true, swap: true, compileHints: true, lazy: ['openpgp', '@tiavision/sinter-mail'], fonts: { preload: ['Figtree Variable', 'Fraunces Variable'] },
      libraries: ['@tiaforge-ui/patina', '@tiavision/sinter-mail'],
      apis: apis('openapi.json'),
    }),
    // temper check on all markup and CSS (errors fail the build); serves the Lucide icons the markup names.
    // Only existing paths: temper stops the build on a missing one (components and layouts are under src/).
    // `inspect`: the dev server also audits the rendered page (overflow, contrast, clipped text, opened dialogs) and
    // prints findings in its terminal; agents check the build with `npm run inspect` instead.
    temper({ paths: ['src', 'index.html', 'calendar', 'contacts', 'mail', 'search', 'settings'].map((p) => resolve(root, p)).filter(existsSync), icons: { lucide: true }, prune: true, inspect: true }),
    // patina check (../patina: patina's data-* vocabulary and --pt-* tokens, through temper's checker) on the same files.
    patina({ paths: ['src', 'index.html', 'calendar', 'contacts', 'mail', 'search', 'settings'].filter((p) => existsSync(resolve(root, p))) }),
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
