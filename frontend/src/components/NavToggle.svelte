<script lang="ts">
  // ☰ button that opens the NavRail drawer on narrow screens; put it first in a temper page's top row.
  import { makeTl, getLocale } from '../lib/i18n.ts';
  const tl = makeTl(getLocale());
</script>

<button type="button" popovertarget="nav" data-show="narrow" data-icon="menu" data-variant="ghost" aria-label={tl('nav_open')}></button>
