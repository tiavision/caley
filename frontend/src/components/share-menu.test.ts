import { expect, test } from 'vitest';
import { render, mockRequests } from '@tiaforge-ui/sinter/testing';
import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from '../test-messages.ts';
import { contactShare, eventShare, type ShareItem } from '../lib/share.ts';
import { app } from '../test-components.ts';
import * as helpers from '../sinter.helpers.ts';
import '../test-dom.ts';                 // jsdom: dialogs, popovers, commandfor

const contact = { account_id: 'a1', uid: 'u 1', addressbook_href: '/ab/', fn_name: 'Ada Lovelace', emails: [], phones: [], addresses: [], org: '', has_photo: false };

const page = `<div data-sn-state="{ item: null }">
  <share-menu data-sn-with="{ item }"></share-menu>
</div>`;

function actions(nativeShare: boolean, log: Array<[string, unknown]>) {
  return {
    share: {
      native: () => nativeShare,
      prepare: async (item: ShareItem) => { log.push(['prepare', item.filename]); return new File(['BEGIN:VCARD'], item.filename, { type: item.type }); },
      send: async (item: ShareItem, file: File | null) => {
        log.push(['send', [item.title, file?.name]]);
        if (item.title === 'Refused') throw Object.assign(new Error('not allowed'), { name: 'NotAllowedError' });
      },
      download: async (item: ShareItem) => { log.push(['download', item]); },
      email: async (item: ShareItem) => { log.push(['email', item]); },
    },
  };
}

async function show(nativeShare: boolean, item: ShareItem) {
  const log: Array<[string, unknown]> = [];
  const html = page.replace('<div data-sn-state="{ item: null }"', `<div data-sn-state="{ item: ${JSON.stringify(item).replace(/"/g, "'")} }"`);
  const r = await render(html, { requests: mockRequests({}), actions: actions(nativeShare, log), helpers, messages, ...app });
  await r.settle();
  return { ...r, log };
}

test('helpers build the file to share', () => {
  expect(contactShare(contact)).toEqual({
    title: 'Ada Lovelace', filename: 'Ada Lovelace.vcf', type: 'text/vcard',
    url: '/api/contact/vcf?account_id=a1&addressbook_href=%2Fab%2F&uid=u+1',
  });
  expect(eventShare({ account_id: 'a1', calendar_href: '/c/', uid: 'e1', summary: 'Lunch: 12/1' } as never).filename).toBe('Lunch_ 12_1.ics');
});

test('with a pointer: the button opens the menu, whose items download or mail the file', async () => {
  const item = contactShare(contact);
  const { root, settle, log } = await show(false, item);
  const button = root.querySelector<HTMLButtonElement>('button[data-icon="share-2"]')!;
  expect(button.getAttribute('popovertarget')).toBe('share-menu');
  expect(button.getAttribute('aria-label')).toBe('Share');
  const [download, email] = root.querySelectorAll<HTMLButtonElement>('#share-menu button');
  expect(download!.textContent).toBe('Download .vCard');
  download!.click();
  email!.click();
  await settle();
  expect(log).toEqual([['download', item], ['email', item]]);
});

test('an event offers the .ics download', async () => {
  const { root } = await show(false, eventShare({ account_id: 'a', calendar_href: '/c/', uid: 'e', summary: 'Lunch' } as never));
  expect(root.querySelector('#share-menu button')!.textContent).toBe('Download .ics');
});

test('on touch screens with Web Share: the file is ready before the tap, which opens the share sheet with it at once', async () => {
  const item = contactShare(contact);
  const { root, settle, log } = await show(true, item);
  expect(log).toEqual([['prepare', 'Ada Lovelace.vcf']]);          // fetched ahead: the sheet opens only right after a tap
  const button = root.querySelector<HTMLButtonElement>('button[data-icon="share-2"]')!;
  expect(button.hasAttribute('popovertarget')).toBe(false);
  button.click();
  await settle();
  expect(log.at(-1)).toEqual(['send', ['Ada Lovelace', 'Ada Lovelace.vcf']]);
  expect(root.querySelector('#share-menu')!.matches(':popover-open')).toBe(false);
});

test('a share sheet the browser refuses opens the menu instead: a tap always shows something', async () => {
  const { root, settle } = await show(true, { ...contactShare(contact), title: 'Refused' });
  root.querySelector<HTMLButtonElement>('button[data-icon="share-2"]')!.click();
  await settle();
  expect(root.querySelector('#share-menu')!.hasAttribute('data-popover-open')).toBe(true);   // showPopover() (src/test-dom.ts)
});
