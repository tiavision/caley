import { loginUrl } from './return-url.ts';

export async function apiFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const res = await fetch(input, init);
  if (res.status === 401) {
    window.location.href = loginUrl();
    return new Promise(() => {});
  }
  return res;
}
