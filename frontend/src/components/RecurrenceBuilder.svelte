<script lang="ts">
  import { makeTl } from '../lib/i18n.ts';

  export let tl: ReturnType<typeof makeTl>;
  export let formScope: string;
  export let fFreq: string;
  export let fInterval: number;
  export let fByDay: string[];
  export let fMonthlyMode: 'by_day' | 'by_setpos';
  export let fMonthday: number;
  export let fSetpos: number;
  export let fSetposDay: string;
  export let fEndType: 'never' | 'count' | 'until';
  export let fCount: number;
  export let fUntil: string;

  $: intervalUnit = (() => {
    const unitMap: Record<string, string> = {
      DAILY: tl('event_repeatUnitDaily'),
      WEEKLY: tl('event_repeatUnitWeekly'),
      MONTHLY: tl('event_repeatUnitMonthly'),
      YEARLY: tl('event_repeatUnitYearly'),
    };
    return unitMap[fFreq] || '';
  })();
</script>

{#if formScope !== 'this_instance'}
  <label>
    <span>{tl('event_repeat')}</span>
    <select bind:value={fFreq}>
      <option value="">{tl('event_repeatNone')}</option>
      <option value="DAILY">{tl('event_repeatDaily')}</option>
      <option value="WEEKLY">{tl('event_repeatWeekly')}</option>
      <option value="MONTHLY">{tl('event_repeatMonthly')}</option>
      <option value="YEARLY">{tl('event_repeatYearly')}</option>
    </select>
  </label>
{:else}
  <p class="repeat-instance-note">{tl('event_repeatInstanceNote')}</p>
{/if}

{#if fFreq}
  <div class="rrule-options">
    <div class="rrule-row">
      <span class="rrule-label">{tl('event_repeatEvery')}</span>
      <div class="rrule-controls">
        <input type="number" min="1" bind:value={fInterval} class="rrule-num-input" />
        <span class="rrule-unit">{intervalUnit}</span>
      </div>
    </div>

    {#if fFreq === 'WEEKLY'}
      <div class="rrule-row">
        <span class="rrule-label">{tl('event_repeatOn')}</span>
        <div class="weekday-toggles">
          {#each [['MO','event_dayMo'],['TU','event_dayTu'],['WE','event_dayWe'],['TH','event_dayTh'],['FR','event_dayFr'],['SA','event_daySa'],['SU','event_daySu']] as [code, lk]}
            <label class="day-toggle">
              <input
                type="checkbox" value={code} checked={fByDay.includes(code)}
                onchange={(e) => {
                  if ((e.target as HTMLInputElement).checked) fByDay = [...fByDay, code];
                  else fByDay = fByDay.filter(d => d !== code);
                }}
              />
              <span>{tl(lk)}</span>
            </label>
          {/each}
        </div>
      </div>
    {/if}

    {#if fFreq === 'MONTHLY'}
      <div class="rrule-row">
        <span class="rrule-label">{tl('event_repeatOn')}</span>
        <div class="monthly-options">
          <label class="monthly-option">
            <input type="radio" name="monthly_mode" value="by_day" bind:group={fMonthlyMode} />
            <span>{tl('event_monthlyByDay')}</span>
            <input type="number" min="1" max="31" bind:value={fMonthday} class="rrule-num-input" disabled={fMonthlyMode !== 'by_day'} />
          </label>
          <label class="monthly-option">
            <input type="radio" name="monthly_mode" value="by_setpos" bind:group={fMonthlyMode} />
            <span>{tl('event_monthlyByPos')}</span>
            <select bind:value={fSetpos} disabled={fMonthlyMode !== 'by_setpos'}>
              <option value={1}>{tl('event_monthlyPos1')}</option>
              <option value={2}>{tl('event_monthlyPos2')}</option>
              <option value={3}>{tl('event_monthlyPos3')}</option>
              <option value={4}>{tl('event_monthlyPos4')}</option>
              <option value={-1}>{tl('event_monthlyPosLast')}</option>
            </select>
            <select bind:value={fSetposDay} disabled={fMonthlyMode !== 'by_setpos'}>
              {#each [['MO','event_dayMo'],['TU','event_dayTu'],['WE','event_dayWe'],['TH','event_dayTh'],['FR','event_dayFr'],['SA','event_daySa'],['SU','event_daySu']] as [code, lk]}
                <option value={code}>{tl(lk)}</option>
              {/each}
            </select>
          </label>
        </div>
      </div>
    {/if}

    <div class="rrule-row">
      <span class="rrule-label">{tl('event_repeatEnd')}</span>
      <div class="rrule-end-options">
        <label class="rrule-end-option">
          <input type="radio" name="rrule_end_type" value="never" bind:group={fEndType} />
          <span>{tl('event_repeatEndNever')}</span>
        </label>
        <label class="rrule-end-option">
          <input type="radio" name="rrule_end_type" value="count" bind:group={fEndType} />
          <span>{tl('event_repeatEndAfter')}</span>
          <input type="number" min="1" bind:value={fCount} class="rrule-num-input" disabled={fEndType !== 'count'} />
          <span>{tl('event_repeatEndAfterN')}</span>
        </label>
        <label class="rrule-end-option">
          <input type="radio" name="rrule_end_type" value="until" bind:group={fEndType} />
          <span>{tl('event_repeatEndOn')}</span>
          <input type="date" bind:value={fUntil} disabled={fEndType !== 'until'} />
        </label>
      </div>
    </div>
  </div>
{/if}

<style>
  .rrule-options { border-left: 3px solid var(--pico-primary-background); padding-left: 0.75rem; margin-bottom: 0.75rem; display: flex; flex-direction: column; gap: 0.625rem; }
  .rrule-row { display: flex; align-items: flex-start; gap: 0.75rem; }
  .rrule-label { font-size: 0.8125rem; text-transform: uppercase; letter-spacing: 0.04em; color: var(--pico-muted-color); font-weight: 600; width: 3.25rem; flex-shrink: 0; padding-top: 0.45rem; }
  .rrule-controls { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
  :global(.rrule-num-input) { width: 4rem !important; margin-bottom: 0 !important; }
  .rrule-unit { font-size: 0.9375rem; }

  .weekday-toggles { display: flex; gap: 0.3rem; flex-wrap: wrap; padding-top: 0.1rem; }
  .day-toggle { position: relative; margin-bottom: 0 !important; display: inline-flex !important; flex-direction: row !important; align-items: center !important; }
  .day-toggle input[type="checkbox"] { position: absolute; opacity: 0; width: 1px; height: 1px; overflow: hidden; }
  .day-toggle span { display: flex; align-items: center; justify-content: center; width: 2.125rem; height: 2.125rem; border-radius: 50%; border: 1.5px solid var(--pico-muted-border-color); font-size: 0.7rem; font-weight: 600; cursor: pointer; user-select: none; transition: background 0.15s, border-color 0.15s, color 0.15s; color: var(--pico-color); background: transparent; letter-spacing: 0; }
  .day-toggle input[type="checkbox"]:checked + span { background: var(--pico-primary-background); border-color: var(--pico-primary-background); color: var(--pico-primary-inverse); }
  .day-toggle span:hover { border-color: var(--pico-primary-background); color: var(--pico-primary-background); }
  .day-toggle input[type="checkbox"]:focus-visible + span { outline: 2px solid var(--pico-primary-background); outline-offset: 2px; }

  .monthly-options { display: flex; flex-direction: column; gap: 0.375rem; }
  .monthly-option { display: flex !important; align-items: center !important; flex-direction: row !important; gap: 0.4rem; margin-bottom: 0 !important; flex-wrap: wrap; }
  .monthly-option input[type="radio"] { flex-shrink: 0; margin: 0; }
  :global(.monthly-option select), :global(.monthly-option .rrule-num-input) { margin-bottom: 0 !important; }

  .rrule-end-options { display: flex; flex-direction: column; gap: 0.375rem; }
  .rrule-end-option { display: flex !important; align-items: center !important; flex-direction: row !important; gap: 0.4rem; margin-bottom: 0 !important; }
  .rrule-end-option input[type="radio"] { flex-shrink: 0; margin: 0; }
  :global(.rrule-end-option input[type="number"]) { width: 4rem !important; margin-bottom: 0 !important; }
  :global(.rrule-end-option input[type="date"]) { flex: 1; min-width: 8rem; margin-bottom: 0 !important; }
  :global(.rrule-end-option input:disabled) { opacity: 0.4; cursor: not-allowed; }

  .repeat-instance-note { font-size: 0.8125rem; color: var(--pico-muted-color); margin: 0 0 0.75rem; font-style: italic; }
</style>
