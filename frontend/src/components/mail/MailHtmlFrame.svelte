<script lang="ts">
  import { createEventDispatcher } from 'svelte';

  export let srcdoc: string;
  export let title: string = '';
  export let scrolling: string | undefined = undefined;
  /** Extra inline styles forwarded directly to the iframe (e.g. height overrides). */
  export let style: string = '';

  let el: HTMLIFrameElement;
  export { el as element };

  const dispatch = createEventDispatcher<{ load: void }>();
</script>

<!-- Sandbox allows same-origin requests (proxy-image) and opening links. -->
<iframe
  bind:this={el}
  sandbox="allow-popups allow-same-origin allow-popups-to-escape-sandbox"
  {srcdoc}
  {title}
  {scrolling}
  {style}
  on:load={() => dispatch('load')}
  class="mail-html-frame"
></iframe>

<style>
  .mail-html-frame {
    width: 100%;
    border: none;
    display: block;
  }
</style>
