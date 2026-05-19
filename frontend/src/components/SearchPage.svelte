<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import EventColorDot from './EventColorDot.svelte';
  import type { SearchResultItem } from '../types/calendar.ts';

  let locale = 'en';
  $: tl = makeTl(locale);

  let query = '';
  let start = '';
  let end = '';
  let results: SearchResultItem[] = [];
  let loading = false;
  let searched = false;
  let errorMsg = '';

  onMount(async () => {
    await initI18n();
    locale = getLocale();
  });

  async function doSearch() {
    const q = query.trim();
    if (!q) return;

    loading = true;
    searched = false;
    errorMsg = '';
    results = [];

    try {
      const params = new URLSearchParams({ q });
      if (start) params.set('start', start);
      if (end) params.set('end', end);

      const res = await fetch(`/api/search?${params}`);
      if (!res.ok) {
        errorMsg = tl('search_error');
      } else {
        results = await res.json();
        searched = true;
      }
    } catch {
      errorMsg = tl('search_error');
    }

    loading = false;
  }

  function formatDate(dtstart: string): string {
    const [y, mo, d] = dtstart.split('-').map(Number);
    if (!y || !mo || !d) return dtstart;
    return new Date(y, mo - 1, d).toLocaleDateString(locale, { dateStyle: 'long' });
  }
</script>

<h2>{tl('search_heading')}</h2>

<form on:submit|preventDefault={doSearch} class="search-form">
  <div class="search-row">
    <input
      type="search"
      bind:value={query}
      placeholder={tl('search_placeholder')}
      autofocus
      class="search-input"
    />
    <button type="submit">{tl('search_submit')}</button>
  </div>
  <div class="date-row">
    <label>
      {tl('search_from')}
      <input type="date" bind:value={start} />
    </label>
    <label>
      {tl('search_to')}
      <input type="date" bind:value={end} />
    </label>
  </div>
</form>

{#if loading}
  <p aria-live="polite">{tl('search_loading')}</p>
{:else if errorMsg}
  <p class="form-error" aria-live="polite">{errorMsg}</p>
{:else if searched && results.length === 0}
  <p aria-live="polite">{tl('search_noResults')}</p>
{:else if results.length > 0}
  <ul class="search-results" aria-live="polite">
    {#each results as item (item.uid + item.dtstart)}
      <li>
        <a href="/?date={item.dtstart}" class="search-result">
          <EventColorDot color={item.calendar_color} />
          <div class="result-body">
            <span class="result-summary">{item.summary}</span>
            <span class="result-meta">
              {formatDate(item.dtstart)}
              {#if item.location} · {item.location}{/if}
            </span>
            {#if item.description}
              <span class="result-desc">{item.description}</span>
            {/if}
            <span class="result-calendar">{item.calendar_name}</span>
          </div>
        </a>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .search-form {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-bottom: 1.5rem;
  }

  .search-row {
    display: flex;
    gap: 0.5rem;
  }

  .search-input {
    flex: 1;
  }

  .date-row {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
  }

  .date-row label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.9em;
  }

  .date-row input[type="date"] {
    padding: 0.3rem 0.5rem;
    font-size: 0.9em;
  }

  .search-results {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .search-result {
    display: flex;
    align-items: flex-start;
    gap: 0.6rem;
    padding: 0.6rem 0.75rem;
    border-radius: var(--pico-border-radius);
    text-decoration: none;
    color: inherit;
    border: 1px solid var(--pico-muted-border-color);
  }

  .search-result:hover {
    background: var(--pico-card-background-color);
    border-color: var(--pico-primary);
  }

  .result-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }

  .result-summary {
    font-weight: 600;
  }

  .result-meta {
    font-size: 0.85em;
    opacity: 0.75;
  }

  .result-desc {
    font-size: 0.85em;
    opacity: 0.7;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .result-calendar {
    font-size: 0.8em;
    opacity: 0.6;
  }
</style>
