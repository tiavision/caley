import { mount } from 'svelte';
import NavRail from '../components/NavRail.svelte';

/** Mounts the nav rail as a direct child of <body data-shell>, before <main>. */
export function initRail() {
  mount(NavRail, { target: document.body, anchor: document.querySelector('body > main') ?? undefined });
}
