import { mount } from 'svelte';
import NavSidebar from '../components/NavSidebar.svelte';
import NavRail from '../components/NavRail.svelte';

/** Pico pages: mounts NavSidebar into #nav. */
export function initNav() {
  // Apply the persisted collapsed state before mount to avoid a width flash.
  try {
    if (localStorage.getItem('nav-collapsed') === '1') {
      document.body.classList.add('nav-collapsed');
    }
  } catch {}

  const target = document.getElementById('nav');
  if (target) mount(NavSidebar, { target });
}

/** temper pages (<body data-shell>): mounts the rail as a direct child of body, before <main>. */
export function initRail() {
  mount(NavRail, { target: document.body, anchor: document.querySelector('body > main') ?? undefined });
}
