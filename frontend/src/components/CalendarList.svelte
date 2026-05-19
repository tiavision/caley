<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import type { CalendarJson } from '../types/calendar.ts';

  export let calendars: CalendarJson[];

  const dispatch = createEventDispatcher<{ toggle: { href: string; active: boolean } }>();
</script>

<ul class="calendar-list">
  {#each calendars as cal}
    <li>
      <label>
        <input
          type="checkbox"
          checked={cal.is_active}
          on:change={(e) => dispatch('toggle', { href: cal.href, active: (e.target as HTMLInputElement).checked })}
        />
        {#if cal.color}
          <span class="dot" style="background:{cal.color}"></span>
        {/if}
        {cal.name}
      </label>
    </li>
  {/each}
</ul>

<style>
  .calendar-list { padding: 0; }
  .calendar-list li { list-style: none; }
  .calendar-list input[type="checkbox"] { cursor: pointer; }
  .dot { border-radius: 50%; width: 10px; height: 10px; display: inline-block; }
</style>
