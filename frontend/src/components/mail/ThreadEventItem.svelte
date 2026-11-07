<script lang="ts">
  import type { EventJson } from '../../types/calendar.ts';
  import { formatEventTime } from '../../lib/format.ts';

  export let event: EventJson;
  export let onclick: () => void;
  export let current: boolean = false;
</script>

<button
  type="button"
  data-variant="item"
  data-size="sm"
  data-layout="row"
  data-gap="sm"
  data-wrap="none"
  data-bar
  style={event.color ? `--tm-tone: ${event.color}` : null}
  aria-current={current ? 'true' : null}
  on:click={onclick}
>
  <span data-icon="calendar" aria-hidden="true"></span>
  <span data-truncate data-grow>{event.summary || '—'}</span>
  <small data-muted>{formatEventTime(event)}</small>
</button>
