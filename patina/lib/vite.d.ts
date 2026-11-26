import type { Plugin } from 'vite';
/** Runs patina check in every build (errors fail it) and on the dev server. */
export default function patina(options?: { paths?: string[]; warningsFail?: boolean }): Plugin;
