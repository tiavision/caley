import { expect, test } from 'vitest';
import { render, mockRequests } from '@tiaforge-ui/sinter/testing';
import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from '../test-messages.ts';
import { calendarValue, importEventDate, importDraft, importSummary } from '../lib/import.ts';
import * as ics from '../actions/ics.ts';
import { app, scripts } from '../test-components.ts';
import '../test-dom.ts';                 // jsdom: dialogs, popovers, commandfor, Blob.stream

// The dialog as mail uses it: a calendar attachment opens it with the file already set (ics.draft from an action),
// a plain button with an empty draft.
const page = `
<body>
  <sn-resource name="caldavAccounts" src="/caldav/accounts"></sn-resource>
  <main data-sn-state="{ defaultCal: '' }">
    <import-dialog>
      <button type="button" id="attachment" data-sn-call="attachment.open(caldavAccounts)" data-sn-then="emit('ics-import', $result)">Attachment</button>
      <button type="button" id="plain" data-sn-on-click="emit('ics-import', importDraft(caldavAccounts, defaultCal, null, null))">Plain</button>
    </import-dialog>
  </main>
</body>`;


const cal = (href: string, name: string) => ({ account_id: 'acc', href, name, color: null, is_active: true });
const accounts = [{
  account_id: 'acc', display_name: 'Home', caldav_base_url: '', caldav_username: 'ada', has_password: true, sort_order: 0,
  calendars: [cal('/cal/work/', 'Work'), cal('/cal/private/', 'Private')], addressbooks: [],
}];

async function show() {
  const requests = mockRequests({
    'GET /api/caldav/accounts': { accounts },
    'POST /api/import': { account_id: 'acc', calendar_href: '/cal/work/', imported: 3, errors: 0 },
  });
  const actions = {
    ics,
    attachment: { open: (a: typeof accounts) => ics.draft(a as never, new File(['x'], 'invite.ics'), { account_id: 'mail', mailbox: 'INBOX', uid: 7 }) },
  };
  const r = await render(page, { requests, actions, helpers: { calendarValue, importEventDate, importDraft, importSummary }, messages, ...app, scripts: { 'close-after': scripts['close-after'] } });
  await r.settle();
  return { ...r, requests };
}

/** A multipart body's field: a FormData (jsdom) or the encoded text. */
function field(body: unknown, name: string): string | undefined {
  const form = body as { get?: (n: string) => unknown };
  if (typeof form?.get === 'function') {
    const v = form.get(name);
    return v == null ? undefined : String(v);
  }
  return String(body).match(new RegExp(`name="${name}"\\r\\n\\r\\n([^\\r]*)\\r\\n`))?.[1];
}

test('the preview marks the events that replace one in the chosen calendar, and previews again for another', async () => {
  const row = (uid: string, summary: string, replaces: boolean) =>
    ({ uid, summary, dtstart: '2026-11-09T09:00', dtend: '2026-11-09T10:00', is_all_day: false, location: '', replaces });
  const requests = mockRequests({
    'GET /api/caldav/accounts': { accounts },
    // the work calendar has "a" already; any other calendar has nothing of the file
    'POST /api/import/preview': (call) => {
      const work = field(call.body, 'calendar_href') === '/cal/work/';
      return [row('a', 'Standup', work), row('b', 'Retro', false)];
    },
  });
  const r = await render(page, { requests, actions: { ics, attachment: { open: () => null } }, helpers: { calendarValue, importEventDate, importDraft, importSummary }, messages, ...app });
  await r.settle();
  const { root, settle } = r;
  root.querySelector<HTMLButtonElement>('#plain')!.click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#import-ics')!;
  const input = dialog.querySelector<HTMLInputElement>('input[type=file]')!;
  Object.defineProperty(input, 'files', { value: [new File(['BEGIN:VCALENDAR'], 'team.ics')], configurable: true });
  input.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  const warnings = () => [...dialog.querySelectorAll('ul[data-list] li')].map((li) =>
    [li.querySelector('strong')!.textContent, li.querySelector('small[data-tone=warning]')?.textContent ?? null]);

  dialog.querySelector<HTMLButtonElement>('#import-ics-preview')!.click();
  await settle();
  const sent = requests.calls.filter((c) => c.path === '/api/import/preview');
  expect(sent).toHaveLength(1);
  expect([field(sent[0]!.body, 'account_id'), field(sent[0]!.body, 'calendar_href')]).toEqual(['acc', '/cal/work/']);
  expect(warnings()).toEqual([['Standup', m.cal_importReplaces()], ['Retro', null]]);

  // another calendar: previewed again for it, nothing replaces there
  const select = dialog.querySelector<HTMLSelectElement>('select')!;
  select.value = 'acc\0/cal/private/';
  select.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  const again = requests.calls.filter((c) => c.path === '/api/import/preview');
  expect(again).toHaveLength(2);
  expect(field(again[1]!.body, 'calendar_href')).toBe('/cal/private/');
  expect(warnings()).toEqual([['Standup', null], ['Retro', null]]);

  // a new calendar: no target is sent
  select.value = '__new__';
  select.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  const last = requests.calls.filter((c) => c.path === '/api/import/preview').at(-1)!;
  expect(field(last.body, 'account_id')).toBeUndefined();
  expect(warnings()).toEqual([['Standup', null], ['Retro', null]]);
});

test('an attachment opens it with the file set; after an import it closes and reopens empty', async () => {
  const { root, settle, requests } = await show();
  const dialog = root.querySelector<HTMLDialogElement>('#import-ics')!;
  expect(dialog.querySelector('form')).toBeNull();           // no draft yet: the form renders with the first one

  root.querySelector<HTMLButtonElement>('#attachment')!.click();
  await settle();
  const input = dialog.querySelector<HTMLInputElement>('input[type=file]')!;
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('small[data-truncate]')!.textContent).toBe('invite.ics');
  expect(input.required).toBe(false);                        // the file comes with the draft, not the input
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(String(requests.calls.at(-2)!.body)).toContain('name="email_uid"');
  expect(dialog.querySelector('[role=status]')!.textContent!.trim()).toBe(m.cal_importSuccess({ n: 3 }));
  await new Promise((resolve) => setTimeout(resolve, 1600));
  expect(dialog.open).toBe(false);

  root.querySelector<HTMLButtonElement>('#plain')!.click();
  await settle();
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('[role=status]')!.textContent!.trim()).toBe('');   // the live region stays, empty
  expect(dialog.querySelector('small[data-truncate]')).toBeNull();
  expect(input.required).toBe(true);
  expect(dialog.querySelector<HTMLSelectElement>('select')!.value).toBe('acc\0/cal/work/');
  await new Promise((resolve) => setTimeout(resolve, 1600));
  expect(dialog.open).toBe(true);                             // the old <close-after> is gone: no timer closes it

  // and once more from the attachment: the file is back
  dialog.close();
  await settle();
  root.querySelector<HTMLButtonElement>('#attachment')!.click();
  await settle();
  expect(dialog.querySelector('small[data-truncate]')!.textContent).toBe('invite.ics');
  expect(input.required).toBe(false);
});
