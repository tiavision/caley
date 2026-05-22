import { m } from '../paraglide/messages.js';
import { getUser } from '../lib/user.ts';

export function initNav() {
  document.getElementById('nav-brand')!.textContent = m.app_brand();
  document.getElementById('nav-loading')!.textContent = m.nav_loading();
  loadNavUser();
}

async function loadNavUser() {
  try {
    const user = await getUser();
    if (!user.ok) {
      document.getElementById('nav-loading')!.textContent = '';
      return;
    }
    const list = document.getElementById('nav-user-list');
    if (!list) return;
    const items: string[] = [];
    if (user.display_name) items.push(`<li>${user.display_name}</li>`);
    if (!user.has_settings) {
      items.push(`<li><a href="/settings/">${m.nav_setup()}</a></li>`);
    } else {
      items.push(`<li><a href="/settings/">${m.nav_settings()}</a></li>`);
    }
    items.push(`<li><a href="/logout">${m.nav_logout()}</a></li>`);
    list.innerHTML = items.join('');
  } catch {}
}