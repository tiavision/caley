<script lang="ts">
  import type { CalendarJson } from '../types/calendar.ts';
  import EventColorDot from './EventColorDot.svelte';

  export let calendars: CalendarJson[];
  export let ontoggle: ((data: { href: string; active: boolean }) => void) | undefined = undefined;
</script>

<ul class="calendar-list">
  {#each calendars as cal}
    <li>
      <label>
        <input
          type="checkbox"
          checked={cal.is_active}
          onchange={(e) => ontoggle?.({ href: cal.href, active: (e.target as HTMLInputElement).checked })}
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
