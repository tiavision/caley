import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { paraglideVitePlugin } from '@inlang/paraglide-js';
import { resolve } from 'path';

export default defineConfig({
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
      },
    },
  },
  server: {
    proxy: {
      '/api': 'http://localhost:3000',
      '/logout': 'http://localhost:3000',
    },
  },
});