// @ts-check
import { defineConfig } from 'astro/config';
import svelte from '@astrojs/svelte';
import { paraglideVitePlugin } from '@inlang/paraglide-js';

// https://astro.build/config
export default defineConfig({
  integrations: [svelte()],
  vite: {
    plugins: [
      paraglideVitePlugin({
        project: './project.inlang',
        outdir: './src/paraglide',
      }),
    ],
    server: {
      proxy: {
        '/api': 'http://localhost:3000',
        '/logout': 'http://localhost:3000',
      },
    },
  },
});
