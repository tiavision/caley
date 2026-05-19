<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import type { CalendarJson } from '../types/calendar.ts';
  import EventColorDot from './EventColorDot.svelte';

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
        <EventColorDot color={cal.color} />
        {cal.name}
      </label>
    </li>
  {/each}
</ul>

<style>
  .calendar-list { padding: 0; }
  .calendar-list li { list-style: none; }
  .calendar-list input[type="checkbox"] { cursor: pointer; }
</style>
