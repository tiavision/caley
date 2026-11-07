import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { paraglideVitePlugin } from '@inlang/paraglide-js';
import { resolve } from 'path';
import temper from '@tiaforge-ui/temper/vite';

const root = import.meta.dirname;

// Files already moved from Pico to temper; `temper check` runs on these during dev and build.
// Remove the list (check everything) once the last page has moved.
const temperPaths: string[] = [
  'src/styles/app.scss',
];

export default defineConfig({
  appType: 'mpa',
  plugins: [
    svelte(),
    temper({ paths: temperPaths.map((p) => resolve(root, p)), icons: { lucide: true } }),
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
