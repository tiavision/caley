// The script of <event-reminders>: timers for the week's event alarms.
import { dateFromEventValue, formatEventTime } from '../lib/format.ts';
import type { DayEventsJson } from '../types/calendar.ts';
import { EventRemindersComponent } from './event-reminders.html';

/** Desktop notifications for the event alarms of the next 24 hours among `days`. */
export class EventReminders extends EventRemindersComponent {
  #timers: ReturnType<typeof setTimeout>[] = [];

  connected() { this.propsChanged(); }

  propsChanged() {
    const days = this.props.days;
    this.#clear();
    if (typeof Notification === 'undefined' || !days?.length) return;
    if (Notification.permission === 'granted') this.#schedule(days);
    else if (Notification.permission === 'default') {
      void Notification.requestPermission().then((p) => { if (p === 'granted' && this.root.isConnected) this.#schedule(days); });
    }
  }

  disconnected() { this.#clear(); }

  #clear() {
    for (const t of this.#timers) clearTimeout(t);
    this.#timers = [];
  }

  #schedule(days: readonly DayEventsJson[]) {
    const now = Date.now(), horizon = now + 86400000;
    const ms = (s: string) => { const d = s ? dateFromEventValue(s) : null; return d ? d.getTime() : NaN; };
    for (const day of days) {
      for (const ev of day.events) {
        for (const alarm of ev.alarms) {
          let fire: number;
          if (alarm.direction === 'absolute') {
            if (!alarm.datetime) continue;
            fire = new Date(alarm.datetime.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/, '$1-$2-$3T$4:$5:$6Z')).getTime();
          } else {
            const base = alarm.direction.endsWith('-end') ? ms(ev.dtend) : ms(ev.dtstart);
            if (isNaN(base)) continue;
            fire = base + (alarm.direction.startsWith('before') ? -1 : 1) * parseInt(alarm.value || '0') * parseInt(alarm.unit || '1') * 60000;
          }
          if (isNaN(fire) || fire <= now || fire > horizon) continue;
          // the tag the server's reminder of this alarm has (backend/src/reminders.rs, pushed to the user's devices):
          // the page's and the pushed one replace each other, as do two pages'
          const tag = `reminder|${ev.uid}|${Math.round(fire / 1000)}`;
          this.#timers.push(setTimeout(() => {
            if (Notification.permission === 'granted') new Notification(ev.summary, { body: formatEventTime(ev), tag });
          }, fire - now));
        }
      }
    }
  }
}
