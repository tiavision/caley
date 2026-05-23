export interface UserInfo {
  ok: boolean;
  status: number;
  display_name?: string;
  has_settings?: boolean;
  has_caldav?: boolean;
  oidc_enabled?: boolean;
  map_url?: string;
}

let cache: Promise<UserInfo> | null = null;

export function getUser(): Promise<UserInfo> {
  if (!cache) {
    cache = fetch('/api/user')
      .then(async r => ({ ok: r.ok, status: r.status, ...(r.ok ? await r.json() : {}) }))
      .catch(() => ({ ok: false, status: 0 }));
  }
  return cache;
}
