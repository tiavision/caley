import { m } from '../paraglide/messages.js';

const navStyles = document.createElement('style');
navStyles.textContent = `
  a.brand { color: inherit; text-decoration: none; }
  nav { flex-wrap: wrap; row-gap: 0.25rem; }
  #nav-user-list { flex-wrap: wrap; }
  #nav-user-list li { padding: 0 0.25rem; }
  #nav-user-list li + li::before { content: "|"; margin-right: 0.5rem; opacity: 0.4; }
  #nav-user-list li:not(:has(a)) { font-weight: 600; opacity: 0.8; }
`;
document.head.appendChild(navStyles);

export function initNav() {
  document.getElementById('nav-brand')!.textContent = m.app_brand();
  document.getElementById('nav-loading')!.textContent = m.nav_loading();
  loadNavUser();
}

async function loadNavUser() {
  try {
    const res = await fetch('/api/user');
    if (!res.ok) {
      document.getElementById('nav-loading')!.textContent = '';
      return;
    }
    const user = await res.json();
    const list = document.getElementById('nav-user-list');
    if (!list) return;
    const items: string[] = [];
    if (user.display_name) items.push(`<li>${user.display_name}</li>`);
    if (!user.has_settings) {
      items.push(`<li><a href="/settings">${m.nav_setup()}</a></li>`);
    } else {
      items.push(`<li><a href="/settings">${m.nav_settings()}</a></li>`);
    }
    items.push(`<li><a href="/logout">${m.nav_logout()}</a></li>`);
    list.innerHTML = items.join('');
  } catch {}
}