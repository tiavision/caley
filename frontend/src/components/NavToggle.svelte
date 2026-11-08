<script lang="ts">
  // ☰ button that opens the nav rail drawer (partials/nav.html) on narrow screens; Svelte pages only; sinter pages include partials/nav-toggle.html.
  import { makeTl, getLocale } from '../lib/i18n.ts';
  const tl = makeTl(getLocale());
</script>

<button type="button" popovertarget="nav" data-show="narrow" data-icon="menu" data-variant="ghost" aria-label={tl('nav_open')}></button>
