import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { paraglideVitePlugin } from '@inlang/paraglide-js';
import { resolve } from 'path';
import temper from '@tiaforge-ui/temper/vite';

const root = import.meta.dirname;


export default defineConfig({
  appType: 'mpa',
  plugins: [
    svelte(),
    // temper check on all markup and CSS (errors fail the build); serves the Lucide icons the markup names.
    temper({ paths: ['src', 'index.html', 'calendar', 'contacts', 'mail', 'search', 'settings'].map((p) => resolve(root, p)), icons: { lucide: true }, prune: true }),
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
