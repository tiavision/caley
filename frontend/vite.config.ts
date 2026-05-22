import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { paraglideVitePlugin } from '@inlang/paraglide-js';
import { resolve } from 'path';

export default defineConfig({
  appType: 'mpa',
  plugins: [
    svelte(),
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
        index: resolve(__dirname, 'index.html'),
        settings: resolve(__dirname, 'settings/index.html'),
        search: resolve(__dirname, 'search/index.html'),
        contacts: resolve(__dirname, 'contacts/index.html'),
        mail: resolve(__dirname, 'mail/index.html'),
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