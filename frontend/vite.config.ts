import { defineConfig } from 'vite';
import { existsSync } from 'fs';
import { resolve } from 'path';
import temper from '@tiaforge-ui/temper/vite';
import sinter from '@tiaforge-ui/sinter/vite';
import { apis } from './apis.ts';

const root = import.meta.dirname;


// what temper checks, emits icons for and keeps rules for when it prunes: the app's markup and the sinter-alloy
// component it uses (its attributes are set through bindings, which temper reads in the markup it is given)
const temperPaths = ['src', 'index.html', 'calendar', 'contacts', 'mail', 'search', 'settings', 'node_modules/@tiavision/sinter-alloy/sinter/components/alloy-sortable.html'].map((p) => resolve(root, p)).filter(existsSync);

export default defineConfig({
  // one sinter for the app and its libraries: sinter-alloy's scripts find the page's app through sinter's `instance()`,
  // which a second copy (the library's own dev install, under a `file:` link) would not know
  resolve: { dedupe: ['@tiaforge-ui/sinter', '@tiaforge-ui/temper'] },
  appType: 'mpa',
  plugins: [
    // Every HTML entry is compiled by sinter (src/components/, src/layouts/, src/sinter.{config,helpers,actions,types,content}.ts). The
    // languages and messages come from the content module (src/sinter.content.ts): messages/{locale}.json of project.inlang.
    // fonts.preload: the UI font (Figtree, @fontsource-variable/figtree) is preloaded, so a first visit does not paint the fallback font first.
    // apis: the backend's JSON API (apis.ts, described by openapi.json).
    // libraries: sinter-mail's <mail-body> (a mail body parsed by its email.parse, src/sinter.helpers.ts) and <mail-width>;
    // alloy's <alloy-sortable> (tiavision/sinter-alloy, a checkout beside caley's: file:../../sinter-alloy; the mail sidebar's virtual folders).
    // lazy: openpgp, and sinter-mail: its parser, its MIME reader and its component scripts (<mail-body>'s builder,
    // <mail-width>, <mail-warm>) load on first use, off the mail pages' bundle; <mail-warm> starts that while the page
    // waits for the message. The actions await every mail.* call.
    // compileHints: V8 compiles each page module as it loads (the mail pages parse a mail in the browser at once).
    sinter({
      typecheck: true, strict: true, embed: true, localePrefix: true, swap: true, compileHints: true, lazy: ['openpgp', '@tiavision/sinter-mail'], fonts: { preload: ['Figtree Variable'] },
      libraries: ['@tiavision/sinter-mail', '@tiavision/sinter-alloy'],
      apis: apis('openapi.json'),
    }),
    // temper check on all markup and CSS (errors fail the build); serves the Lucide icons the markup names.
    // Only existing paths: temper stops the build on a missing one (components and layouts are under src/).
    // `inspect`: the dev server also audits the rendered page (overflow, contrast, clipped text, opened dialogs) and
    // prints findings in its terminal; agents check the build with `npm run inspect` instead.
    // alloy's components are checked too, and the icons they name (grip-vertical) served: temper reads only these paths,
    // not sinter's libraries.
    temper({ paths: temperPaths, icons: { lucide: true }, prune: { paths: temperPaths }, inspect: true }),
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
