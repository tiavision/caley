import { mount } from 'svelte';
import NavSidebar from '../components/NavSidebar.svelte';

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
