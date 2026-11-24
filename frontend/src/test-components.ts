/// <reference types="vite/client" />
import { apis } from '../apis.ts';
import spec from '../openapi.json';
import { MailWidthComponent } from '../node_modules/@tiavision/sinter-mail/sinter/components/mail-width.html';
// The app's components and the layout, for page tests: render(page, { ...app, … }).
// Without `scripts` the components render without their scripts (jsdom measures no layout and has no
// ResizeObserver); a test that needs one passes it: render(page, { ...app, scripts: { 'close-after': scripts['close-after'] } }).
const html = import.meta.glob<string>('./components/*.html', { query: '?raw', import: 'default', eager: true });
const ts = import.meta.glob(['./components/*.ts', '!./components/*.test.ts'], { eager: true });
// sinter-mail's components (<mail-body> and its internal ones), which the build takes from sinter({ libraries }):
// render() has no option for a component library, so they are passed as components of the app.
const library = import.meta.glob<string>('../node_modules/@tiavision/sinter-mail/sinter/components/**/*.html', { query: '?raw', import: 'default', eager: true });
// and their scripts: <mail-body> renders a body from data through its script builder (mail-view.ts), without which it
// renders nothing. jsdom lays nothing out and has no ResizeObserver, so <mail-width> reports a wide reader at once
// (`mailWidth('narrow')`: a phone's).
const libraryScripts = import.meta.glob(['../node_modules/@tiavision/sinter-mail/sinter/components/**/*.ts', '!**/mail-width.ts'], { eager: true });
const layouts = import.meta.glob<string>('./layouts/*.html', { query: '?raw', import: 'default', eager: true });

const name = (path: string) => path.replace(/^.*\//, '').replace(/\.(html|ts)$/, '');

export const components: Record<string, string> = Object.fromEntries([...Object.entries(library), ...Object.entries(html)].map(([p, t]) => [name(p), t]));
export const scripts: Record<string, unknown> = Object.fromEntries(Object.entries(ts).map(([p, mod]) => [name(p), mod]));
export const layoutFiles: Record<string, string> = Object.fromEntries(Object.entries(layouts).map(([p, t]) => [name(p), t]));

/** <mail-width>'s script for page tests: it reports `media` once it is in the page. */
export function mailWidth(media: 'wide' | 'narrow') {
  return { MailWidth: class extends MailWidthComponent { connected() { this.emit('media', media); } } };
}

// the build's APIs, with the spec (paths in the markup are relative to /api: mocks still name the full URL)
// sinter-mail is a lazy package (vite.config.ts): its helper namespace `email` (src/sinter.helpers.ts) reaches the pages
// as a helper that waits, which render() takes as `lazyHelpers`; here it loads at once.
const lazyHelpers = {
  email: {
    members: ['parse', 'plain', 'text', 'links', 'unquoted', 'progressive', 'mime', 'bytes', 'imageType', 'forget'],
    load: () => import('@tiavision/sinter-mail'),
  },
};

export const app = {
  components, layouts: layoutFiles, apis: apis(spec), lazyHelpers,
  scripts: { ...Object.fromEntries(Object.entries(libraryScripts).map(([p, mod]) => [name(p), mod])), 'mail-width': mailWidth('wide') },
};
