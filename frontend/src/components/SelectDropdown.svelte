<script lang="ts">
  export let options: { value: string; label: string; secondary?: string }[] = [];
  export let value: string = '';

  let open = false;
  $: selected = options.find(o => o.value === value);

  function pick(v: string) {
    value = v;
    open = false;
  }
</script>

<div class="sd-wrap">
  <button type="button" class="sd-trigger" on:click|stopPropagation={() => open = !open}>
    <span class="sd-value">
      {#if selected}
        {selected.label}{selected.secondary ? ' ' + selected.secondary : ''}
      {/if}
    </span>
    <svg class="sd-chevron" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 9 12 15 18 9"/></svg>
  </button>
  {#if open}
    <div class="sd-overlay" on:click={() => open = false}></div>
    <ul class="sd-list" role="listbox">
      {#each options as opt}
        <li role="option" aria-selected={opt.value === value}>
          <button type="button" class="sd-option" class:selected={opt.value === value}
            on:click={() => pick(opt.value)}>
            <span class="sd-opt-label">{opt.label}</span>
            {#if opt.secondary}<span class="sd-opt-secondary">{opt.secondary}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .sd-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
  }

  .sd-trigger {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    width: 100%;
    background: none;
    border: none;
    padding: 0.45rem 0;
    margin: 0;
    font-size: inherit;
    color: inherit;
    cursor: pointer;
    text-align: left;
  }

  .sd-trigger:focus {
    outline: none;
    box-shadow: none;
  }

  .sd-value {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sd-chevron {
    flex-shrink: 0;
    opacity: 0.55;
  }

  .sd-overlay {
    position: fixed;
    inset: 0;
    z-index: 199;
  }

  .sd-list {
    position: absolute;
    top: 100%;
    left: -1rem;
    right: -1rem;
    z-index: 200;
    list-style: none;
    padding: 0.25rem 0;
    margin: 0;
    background: var(--pico-card-background-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: 0.375rem;
    box-shadow: 0 0.25rem 0.75rem rgba(0, 0, 0, 0.15);
  }

  /* Pico sets `ul li { list-style: square }` directly on li, overriding inherited none */
  .sd-list li {
    list-style: none;
  }

  .sd-option {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    width: 100%;
    background: none;
    border: none;
    padding: 0.45rem 1rem;
    margin: 0;
    font-size: inherit;
    color: var(--pico-color);
    cursor: pointer;
    text-align: left;
  }

  .sd-option:hover,
  .sd-option.selected {
    background: var(--pico-secondary-background);
  }

  .sd-opt-label {
    white-space: nowrap;
  }

  .sd-opt-secondary {
    color: var(--pico-muted-color);
    font-size: smaller;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
