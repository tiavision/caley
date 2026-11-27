import { expect, test } from 'vitest';
import { render, mockRequests } from '@tiaforge-ui/sinter/testing';
import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from '../test-messages.ts';
import { calendarValue, importEventDate, importDraft, importSummary } from '../lib/import.ts';
import * as ics from '../actions/ics.ts';
import { app, scripts } from '../test-components.ts';
import '../test-dom.ts';                 // jsdom: dialogs, popovers, commandfor, Blob.stream

/** a multipart body's field names: a FormData of any realm (jsdom's or Node's: no `instanceof`), or its text */
const fieldNames = (body: unknown): string[] => Object.prototype.toString.call(body) === '[object FormData]'
  ? [...(body as FormData).keys()]
  : [...String(body).matchAll(/name="([^"]+)"/g)].map((x) => x[1]!);

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

test('an attachment opens it with the file set; after an import it closes and reopens empty', async () => {
  const { root, settle, requests } = await show();
  const dialog = root.querySelector<HTMLDialogElement>('#import-ics')!;
  expect(dialog.querySelector('form')).toBeNull();           // no draft yet: the form renders with the first one

  root.querySelector<HTMLButtonElement>('#attachment')!.click();
  await settle();
  const input = dialog.querySelector<HTMLInputElement>('input[type=file]')!;
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('small[data-tm-truncate]')!.textContent).toBe('invite.ics');
  expect(input.required).toBe(false);                        // the file comes with the draft, not the input
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(fieldNames(requests.calls.at(-2)!.body)).toContain('email_uid');
  expect(dialog.querySelector('[role=status]')!.textContent!.trim()).toBe(m.cal_importSuccess({ n: 3 }));
  await new Promise((resolve) => setTimeout(resolve, 1600));
  expect(dialog.open).toBe(false);

  root.querySelector<HTMLButtonElement>('#plain')!.click();
  await settle();
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('[role=status]')!.textContent!.trim()).toBe('');   // the live region stays, empty
  expect(dialog.querySelector('small[data-tm-truncate]')).toBeNull();
  expect(input.required).toBe(true);
  expect(dialog.querySelector<HTMLSelectElement>('select')!.value).toBe('acc\0/cal/work/');
  await new Promise((resolve) => setTimeout(resolve, 1600));
  expect(dialog.open).toBe(true);                             // the old <close-after> is gone: no timer closes it

  // and once more from the attachment: the file is back
  dialog.close();
  await settle();
  root.querySelector<HTMLButtonElement>('#attachment')!.click();
  await settle();
  expect(dialog.querySelector('small[data-tm-truncate]')!.textContent).toBe('invite.ics');
  expect(input.required).toBe(false);
});
