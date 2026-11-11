/// <reference types="vite/client" />
// The app's components and the layout, for page tests: render(page, { ...app, … }).
// Without `scripts` the components render without their scripts (jsdom measures no layout and has no
// ResizeObserver); a test that needs one passes it: render(page, { ...app, scripts: { 'close-after': scripts['close-after'] } }).
const html = import.meta.glob<string>('./components/*.html', { query: '?raw', import: 'default', eager: true });
const ts = import.meta.glob(['./components/*.ts', '!./components/*.test.ts'], { eager: true });
const layouts = import.meta.glob<string>('./layouts/*.html', { query: '?raw', import: 'default', eager: true });

const name = (path: string) => path.replace(/^.*\//, '').replace(/\.(html|ts)$/, '');

export const components: Record<string, string> = Object.fromEntries(Object.entries(html).map(([p, t]) => [name(p), t]));
export const scripts: Record<string, unknown> = Object.fromEntries(Object.entries(ts).map(([p, mod]) => [name(p), mod]));
export const layoutFiles: Record<string, string> = Object.fromEntries(Object.entries(layouts).map(([p, t]) => [name(p), t]));

export const app = { components, layouts: layoutFiles };
