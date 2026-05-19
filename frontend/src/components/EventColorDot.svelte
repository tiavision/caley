<script lang="ts">
  export let color: string | null | undefined;
</script>

{#if color}
  <span class="dot" style="background:{color}"></span>
{/if}

<style>
  .dot {
    border-radius: 50%;
    width: 10px;
    height: 10px;
    display: inline-block;
    flex-shrink: 0;
  }
</style>
