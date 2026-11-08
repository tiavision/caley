// TEMPORARY: the Svelte thread and event dialogs on the sinter mail pages, until their sinter
// partials (thread dialog, event detail: search+calendar) are committed. The reader emits `mail-threads`; picking a message in the thread dialog opens it in the reader.
import { mount } from 'svelte';
import { instance } from '@tiaforge-ui/sinter';
import ThreadDialog from '../components/mail/ThreadDialog.svelte';
import EventDetail from '../components/EventDetail.svelte';
import { apiFetch } from '../lib/api.ts';
import { getLocale } from '../lib/i18n.ts';
import type { EventJson, MessageEventGroupJson, SourceEmailJson } from '../types/calendar.ts';
import type { Message, ThreadNode } from '../lib/mail.ts';

const host = document.createElement('div');
document.body.append(host);
const locale = getLocale();
const key = (a: string, b: string, c: number) => `${a} ${b} ${c}`;
const groupsByKey = (groups: MessageEventGroupJson[]) =>
  Object.fromEntries(groups.map((g) => [key(g.account_id, g.mailbox, g.uid), g.events]));

function openInReader(node: ThreadNode) {
  instance(document).state(document.body).update((s: Record<string, unknown>) => {
    s.open = { account_id: node.account_id, mailbox: node.mailbox, uid: node.uid };
    s.detailOpen = true;
  });
}

const props = $state({
  locale,
  threadMessages: [] as ThreadNode[],
  eventsByMessageKey: {} as Record<string, EventJson[]>,
  highlightedUid: undefined as { account_id: string; mailbox: string; uid: number } | undefined,
  currentEventUid: undefined as string | undefined,
  onmessageclick: (node: ThreadNode) => { thread.close(); openInReader(node); },
  oneventclick: (ev: EventJson) => { thread.close(); detail.open(ev); },
});
const thread = mount(ThreadDialog, { target: host, props }) as unknown as { show(): void; close(): void };
const detail = mount(EventDetail, {
  target: host,
  props: {
    locale,
    onthreads: (emails: SourceEmailJson[], event: EventJson) => { detail.close(); void eventThread(emails, event); },
  },
}) as unknown as { open(ev: EventJson): void; close(): void };

async function eventThread(emails: SourceEmailJson[], event: EventJson) {
  const src = emails[0];
  if (!src) return;
  const q = `account_id=${encodeURIComponent(src.account_id)}&mailbox=${encodeURIComponent(src.mailbox)}&uid=${src.uid}`;
  const [t, e] = await Promise.all([apiFetch(`/api/email/thread?${q}`), apiFetch(`/api/email/thread-events?${q}`)]);
  props.threadMessages = t.ok ? (await t.json()).messages ?? [] : [];
  props.eventsByMessageKey = e.ok ? groupsByKey(await e.json()) : {};
  props.currentEventUid = event.uid;
  thread.show();
}

document.addEventListener('mail-threads', (e) => {
  const { messages, groups, current } = (e as CustomEvent<{ messages: ThreadNode[]; groups: MessageEventGroupJson[]; current: Message }>).detail;
  props.threadMessages = messages;
  props.eventsByMessageKey = groupsByKey(groups);
  props.currentEventUid = undefined;
  props.highlightedUid = { account_id: current.account_id, mailbox: current.mailbox, uid: current.uid };
  thread.show();
});

