<script lang="ts">
  export let kind: string;
</script>

{#if kind === 'inbox'}
  <span data-icon="inbox" aria-hidden="true"></span>
{:else if kind === 'sent'}
  <span data-icon="send" aria-hidden="true"></span>
{:else if kind === 'drafts'}
  <span data-icon="pencil" aria-hidden="true"></span>
{:else if kind === 'trash'}
  <span data-icon="trash-2" aria-hidden="true"></span>
{:else if kind === 'junk'}
  <span data-icon="octagon-alert" aria-hidden="true"></span>
{:else if kind === 'archive'}
  <span data-icon="archive" aria-hidden="true"></span>
{:else if kind === 'virtual'}
  <span data-icon="layers" aria-hidden="true"></span>
{:else}
  <span data-icon="folder" aria-hidden="true"></span>
{/if}
