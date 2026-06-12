<script lang="ts">
  import type { EventJson } from '../../types/calendar.ts';
  import { formatEventTime } from '../../lib/format.ts';

  export let event: EventJson;
  export let onclick: () => void;
  export let current: boolean = false;
</script>

<button class="thread-event" class:current on:click={onclick} type="button">
  <span class="thread-event-icon" aria-hidden="true">
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/>
      <line x1="16" y1="2" x2="16" y2="6"/>
      <line x1="8" y1="2" x2="8" y2="6"/>
      <line x1="3" y1="10" x2="21" y2="10"/>
    </svg>
  </span>
  <span class="thread-event-summary">{event.summary || '—'}</span>
  <span class="thread-event-time">{formatEventTime(event)}</span>
</button>

<style>
  .thread-event {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    width: 100%;
    padding: 0.35rem 0.6rem;
    border: none;
    border-radius: 4px;
    background: color-mix(in srgb, var(--pico-primary-background) 12%, transparent);
    color: var(--pico-color);
    cursor: pointer;
    font-size: 0.85rem;
    text-align: left;
    transition: background 0.15s;
  }

  .thread-event:hover {
    background: color-mix(in srgb, var(--pico-primary-background) 22%, transparent);
  }

  .thread-event.current {
    background: var(--pico-primary-background, #e6f2fb);
    box-shadow: inset 3px 0 0 var(--pico-primary, #0172ad);
  }

  .thread-event-icon {
    flex-shrink: 0;
    color: var(--pico-primary);
    display: flex;
    align-items: center;
  }

  .thread-event-summary {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }

  .thread-event-time {
    flex-shrink: 0;
    color: var(--pico-muted-color);
    font-size: 0.8rem;
    white-space: nowrap;
  }
</style>
