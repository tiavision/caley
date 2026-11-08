import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
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
    // Pages not migrated yet still mount their Svelte app into #app from their entry script.
    sinter({ typecheck: true }),
    svelte(),
    // temper check on all markup and CSS (errors fail the build); serves the Lucide icons the markup names.
    // Only existing paths: temper stops the build on a missing one (partials/ arrives with the sinter pages).
    temper({ paths: ['src', 'partials', 'index.html', 'calendar', 'contacts', 'mail', 'search', 'settings'].map((p) => resolve(root, p)).filter(existsSync), icons: { lucide: true }, prune: true }),
    paraglideVitePlugin({
      project: './project.inlang',
      outdir: './src/paraglide',
    }),
    // Pages render in JS: make the entry script render-blocking so a cross-document view transition
    // (app.scss) captures the mounted page, not an empty <main>. Vite drops the attribute from the source tag.
    {
      name: 'caley-render-blocking-entry',
      transformIndexHtml: {
        order: 'post',
        handler: (html) => html.replace(/<script type="module"(?![^>]*\bblocking=)/g, '<script type="module" blocking="render"'),
      },
    },
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
