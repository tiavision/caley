<script lang="ts">
  export let name: 'import' | 'export' | 'trash' | 'key-plus' | 'identity-plus' | 'lock' | 'signature' | 'verified' | 'tree';
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
  {:else if name === 'export'}
    <path d="M12 4v12" />
    <polyline points="7 11 12 16 17 11" />
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
  {:else if name === 'identity-plus'}
    <circle cx="9" cy="7" r="4" />
    <path d="M2 19c0-4 3.1-7 7-7 2.1 0 4 0.9 5.3 2.3" />
    <path d="M19 15v6" />
    <path d="M16 18h6" />
  {:else if name === 'lock'}
    <rect x="5" y="10" width="14" height="10" rx="2" />
    <path d="M8 10V7a4 4 0 0 1 8 0v3" />
    <path d="M12 14v2" />
  {:else if name === 'signature'}
    <path d="M3 18c3.4-5.3 5.2-11.8 7.3-11.8 3.4 0-1.6 10.3-1.1 11.3.7 1.5 3.4-4.1 4.5-4.1 1.1 0 .3 3.7 1.5 3.7 1 0 2-2 3.2-2 1.1 0 1.6.6 2.6 1.2" />
    <path d="M3 21h18" />
  {:else if name === 'verified'}
    <circle cx="12" cy="12" r="9" />
    <polyline points="8 12 11 15 16 9" />
  {:else if name === 'tree'}
    <path d="M4 5v14" />
    <path d="M4 9h5" />
    <path d="M4 16h5" />
    <rect x="9" y="6" width="11" height="6" rx="1.5" />
    <rect x="9" y="13" width="11" height="6" rx="1.5" />
  {/if}
</svg>
