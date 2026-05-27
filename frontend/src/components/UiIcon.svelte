<script lang="ts">
  export let name: 'import' | 'trash' | 'key-plus';
  export let size = 16;
  export let strokeWidth = 2;
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === 'import'}
    <path d="M12 16V4" />
    <polyline points="7 9 12 4 17 9" />
    <path d="M4 17v3h16v-3" />
  {:else if name === 'trash'}
    <polyline points="3 6 5 6 21 6" />
    <path d="M19 6l-1 14H6L5 6" />
    <path d="M10 11v6" />
    <path d="M14 11v6" />
    <path d="M9 6V4h6v2" />
  {:else if name === 'key-plus'}
    <circle cx="8" cy="15" r="4" />
    <path d="M11 12l8-8" />
    <path d="M15 4h4v4" />
    <path d="M18 15v6" />
    <path d="M15 18h6" />
  {/if}
</svg>
