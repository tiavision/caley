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
    {tl('event_repeat')}
    <select bind:value={fFreq}>
      <option value="">{tl('event_repeatNone')}</option>
      <option value="DAILY">{tl('event_repeatDaily')}</option>
      <option value="WEEKLY">{tl('event_repeatWeekly')}</option>
      <option value="MONTHLY">{tl('event_repeatMonthly')}</option>
      <option value="YEARLY">{tl('event_repeatYearly')}</option>
    </select>
  </label>
{:else}
  <small data-muted>{tl('event_repeatInstanceNote')}</small>
{/if}

{#if fFreq}
  <div data-layout="stack" data-gap="md">
    <fieldset>
      <legend>{tl('event_repeatEvery')}</legend>
      <div data-layout="row" data-gap="sm" data-wrap="none">
        <input type="number" min="1" bind:value={fInterval} class="num" aria-label={tl('event_repeatEvery')} />
        <span>{intervalUnit}</span>
      </div>
    </fieldset>

    {#if fFreq === 'WEEKLY'}
      <fieldset>
        <legend>{tl('event_repeatOn')}</legend>
        <div data-layout="row" data-gap="xs">
          {#each [['MO','event_dayMo'],['TU','event_dayTu'],['WE','event_dayWe'],['TH','event_dayTh'],['FR','event_dayFr'],['SA','event_daySa'],['SU','event_daySu']] as [code, lk]}
            <button
              type="button"
              data-variant="outline"
              data-size="sm"
              aria-pressed={fByDay.includes(code)}
              onclick={() => {
                if (!fByDay.includes(code)) fByDay = [...fByDay, code];
                else fByDay = fByDay.filter(d => d !== code);
              }}
            >{tl(lk)}</button>
          {/each}
        </div>
      </fieldset>
    {/if}

    {#if fFreq === 'MONTHLY'}
      <fieldset>
        <legend>{tl('event_repeatOn')}</legend>
        <div data-layout="row" data-gap="sm">
          <label><input type="radio" name="monthly_mode" value="by_day" bind:group={fMonthlyMode} /> {tl('event_monthlyByDay')}</label>
          <input type="number" min="1" max="31" bind:value={fMonthday} class="num" aria-label={tl('event_monthlyByDay')} disabled={fMonthlyMode !== 'by_day'} />
        </div>
        <div data-layout="row" data-gap="sm">
          <label><input type="radio" name="monthly_mode" value="by_setpos" bind:group={fMonthlyMode} /> {tl('event_monthlyByPos')}</label>
          <select bind:value={fSetpos} aria-label={tl('event_monthlyByPos')} disabled={fMonthlyMode !== 'by_setpos'}>
            <option value={1}>{tl('event_monthlyPos1')}</option>
            <option value={2}>{tl('event_monthlyPos2')}</option>
            <option value={3}>{tl('event_monthlyPos3')}</option>
            <option value={4}>{tl('event_monthlyPos4')}</option>
            <option value={-1}>{tl('event_monthlyPosLast')}</option>
          </select>
          <select bind:value={fSetposDay} aria-label={tl('event_repeatOn')} disabled={fMonthlyMode !== 'by_setpos'}>
            {#each [['MO','event_dayMo'],['TU','event_dayTu'],['WE','event_dayWe'],['TH','event_dayTh'],['FR','event_dayFr'],['SA','event_daySa'],['SU','event_daySu']] as [code, lk]}
              <option value={code}>{tl(lk)}</option>
            {/each}
          </select>
        </div>
      </fieldset>
    {/if}

    <fieldset>
      <legend>{tl('event_repeatEnd')}</legend>
      <label><input type="radio" name="rrule_end_type" value="never" bind:group={fEndType} /> {tl('event_repeatEndNever')}</label>
      <div data-layout="row" data-gap="sm">
        <label><input type="radio" name="rrule_end_type" value="count" bind:group={fEndType} /> {tl('event_repeatEndAfter')}</label>
        <input type="number" min="1" bind:value={fCount} class="num" aria-label={tl('event_repeatEndAfterN')} disabled={fEndType !== 'count'} />
        <span>{tl('event_repeatEndAfterN')}</span>
      </div>
      <div data-layout="row" data-gap="sm">
        <label><input type="radio" name="rrule_end_type" value="until" bind:group={fEndType} /> {tl('event_repeatEndOn')}</label>
        <input type="date" bind:value={fUntil} aria-label={tl('event_repeatEndOn')} data-grow disabled={fEndType !== 'until'} />
      </div>
    </fieldset>
  </div>
{/if}

<style>
  .num { inline-size: 5rem; }
</style>
