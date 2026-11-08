import { expect, test } from 'vitest';
import { render, mockRequests } from '@tiaforge-ui/sinter/testing';
import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from '../src/test-messages.ts';
import { calendarValue, importEventDate, importDraft, importSummary } from '../src/lib/import.ts';
import * as ics from '../src/actions/ics.ts';
import '../src/islands/common.ts';                 // <close-after>
import importDialog from './import-dialog.html?raw';

// The dialog as mail uses it: a calendar attachment opens it with the file already set (ics.draft from an action),
// a plain button with an empty draft.
const page = `
<body data-sn-state="{ imp: /** @type {ImportDraft|null} */ (null) }">
  <sn-resource name="caldavAccounts" src="/api/caldav/accounts" items="accounts" key="account_id" type="CalDavAccountJson[]"></sn-resource>
  <main data-sn-state="{ defaultCal: '' }">
    <button type="button" id="attachment" data-sn-call="attachment.open(caldavAccounts)" data-sn-then="imp = $result; showModal('import-ics')">Attachment</button>
    <button type="button" id="plain" data-sn-on-click="imp = importDraft(caldavAccounts, defaultCal, null, null); showModal('import-ics')">Plain</button>
    <sn-include src="./import-dialog.html"></sn-include>
  </main>
</body>`;

if (!HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function () { this.open = true; this.dispatchEvent(new Event('toggle')); };
  HTMLDialogElement.prototype.close = function () { if (!this.open) return; this.open = false; this.dispatchEvent(new Event('toggle')); this.dispatchEvent(new Event('close')); };
}

const cal = (href: string, name: string) => ({ account_id: 'acc', href, name, color: null, is_active: true });
const accounts = [{
  account_id: 'acc', display_name: 'Home', caldav_base_url: '', caldav_username: 'ada', has_password: true, sort_order: 0,
  calendars: [cal('/cal/work/', 'Work'), cal('/cal/private/', 'Private')], addressbooks: [],
}];

async function show() {
  const requests = mockRequests({ 'GET /api/caldav/accounts': { accounts }, 'POST /api/import': { imported: 3, errors: 0 } });
  const actions = {
    ics,
    attachment: { open: (a: typeof accounts) => ics.draft(a as never, new File(['x'], 'invite.ics'), { account_id: 'mail', mailbox: 'INBOX', uid: 7 }) },
  };
  const r = await render(page, { requests, actions, helpers: { calendarValue, importEventDate, importDraft, importSummary }, messages, files: { './import-dialog.html': importDialog } });
  await r.settle();
  return { ...r, requests };
}

test('an attachment opens it with the file set; after an import it closes and reopens empty', async () => {
  const { root, settle, requests } = await show();
  const dialog = root.querySelector<HTMLDialogElement>('#import-ics')!;
  const input = dialog.querySelector<HTMLInputElement>('input[type=file]')!;

  root.querySelector<HTMLButtonElement>('#attachment')!.click();
  await settle();
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('small[data-truncate]')!.textContent).toBe('invite.ics');
  expect(input.required).toBe(false);                        // the file comes with the draft, not the input
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(String(requests.calls.at(-2)!.body)).toContain('name="email_uid"');
  expect(dialog.querySelector('[role=status]')!.textContent).toBe(m.cal_importSuccess({ n: 3 }));
  expect(dialog.querySelector('close-after')).not.toBeNull();
  await new Promise((resolve) => setTimeout(resolve, 1600));
  expect(dialog.open).toBe(false);

  root.querySelector<HTMLButtonElement>('#plain')!.click();
  await settle();
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('[role=status]')).toBeNull();
  expect(dialog.querySelector('close-after')).toBeNull();
  expect(dialog.querySelector('small[data-truncate]')).toBeNull();
  expect(input.required).toBe(true);
  expect(dialog.querySelector<HTMLSelectElement>('select')!.value).toBe('acc\0/cal/work/');
  await new Promise((resolve) => setTimeout(resolve, 1600));
  expect(dialog.open).toBe(true);                             // the old <close-after> is gone

  // and once more from the attachment: the file is back
  dialog.close();
  await settle();
  root.querySelector<HTMLButtonElement>('#attachment')!.click();
  await settle();
  expect(dialog.querySelector('small[data-truncate]')!.textContent).toBe('invite.ics');
  expect(input.required).toBe(false);
});
