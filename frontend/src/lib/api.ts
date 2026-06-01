import { rememberCurrentAppUrl } from './return-url.ts';

export async function apiFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const res = await fetch(input, init);
  if (res.status === 401) {
    rememberCurrentAppUrl();
    window.location.href = '/';
    return new Promise(() => {});
  }
  return res;
}
