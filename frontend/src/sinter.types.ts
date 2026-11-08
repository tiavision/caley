// Types that `type="…"` on <sn-resource> and <sn-event> may name (`sinter check` runs in `vite build`).
// Re-export the app's existing types rather than redefining them.
export type { User } from './lib/user.ts';
export type { NotificationStatus } from './actions/notify.ts';
export type { ShareItem } from './lib/share.ts';
// Re-export each type by name (sinter reads the exported names; a star re-export of types is not seen).

/** GET /api/auth/config (public): what the login page offers. */
export interface AuthConfig {
  oidc_enabled: boolean;
}
