<script lang="ts">
  export let color: string | null | undefined;
</script>

{#if color}
  <span class="dot" style="--tm-tone: {color}"></span>
{/if}

<style>
  /* A calendar-colored dot; temper has no color swatch. */
  .dot {
    display: inline-block;
    flex-shrink: 0;
    inline-size: 10px;
    block-size: 10px;
    border-radius: 50%;
    background: var(--tm-tone);
  }
</style>
