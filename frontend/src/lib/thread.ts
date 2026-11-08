// Pure helpers for partials/thread-dialog.html: the conversation of a message as indented rows, with the
// calendar events imported from each message under it. Exported to templates through src/sinter.helpers.ts.
import type { EventJson, MessageEventGroupJson } from '../types/calendar.ts';
import type { ThreadNode, ThreadTreeNode } from '../components/mail/MailTypes.ts';
import { buildThreadTree, identityKey, senderName, formatDate } from '../components/mail/mailUtils.ts';

/** Which conversation the thread dialog shows: the message it is opened for, and what to mark in it. */
export interface ThreadRef {
  account_id: string;
  mailbox: string;
  uid: number;
  /** mark this message as the one being read (mail) */
  current?: boolean;
  /** mark this event as the one the dialog was opened from (an event's threads button) */
  event_uid?: string | null;
}

/** GET /api/email/thread */
export interface ThreadResponse {
  messages: ThreadNode[];
}

/** One row of the dialog: a message (`node`, `from`, `date`) or an event imported from the message above (`event`). */
export interface ThreadRow {
  kind: 'message' | 'event';
  key: string;
  depth: number;
  current: boolean;
  node?: ThreadNode;
  from?: string;
  date?: string;
  event?: EventJson;
}

function nodeEvents(node: ThreadNode, byKey: Map<string, EventJson[]>, single: boolean): EventJson[] {
  const exact = byKey.get(identityKey(node.account_id, node.mailbox, node.uid)) ?? [];
  if (exact.length) return exact;
  const sameUid = [...byKey].filter(([k]) => k.endsWith(`\0${node.uid}`)).flatMap(([, evs]) => evs);
  if (sameUid.length) return sameUid;
  return single ? [...byKey.values()].flat() : [];
}

/**
 * The thread as rows in reading order: each message, then the events imported from it, then its replies,
 * one level deeper each. `ref` marks the current message and event.
 */
export function threadRows(messages: readonly ThreadNode[], groups: readonly MessageEventGroupJson[], ref: ThreadRef | null): ThreadRow[] {
  const byKey = new Map(groups.map((g) => [identityKey(g.account_id, g.mailbox, g.uid), g.events] as const));
  const rows: ThreadRow[] = [];
  const walk = (branches: ThreadTreeNode[], depth: number) => {
    for (const { message: node, children } of branches) {
      const key = identityKey(node.account_id, node.mailbox, node.uid);
      rows.push({
        kind: 'message', key, depth, node, from: senderName(node.from), date: formatDate(node.sent_at),
        current: !!ref?.current && ref.account_id === node.account_id && ref.mailbox === node.mailbox && ref.uid === node.uid,
      });
      for (const ev of nodeEvents(node, byKey, messages.length === 1)) {
        rows.push({
          kind: 'event', key: `${key}\0${ev.account_id}\0${ev.calendar_href}\0${ev.uid}`, depth: depth + 1, event: ev,
          current: !!ref?.event_uid && ref.event_uid === ev.uid,
        });
      }
      walk(children, depth + 1);
    }
  };
  walk(buildThreadTree([...messages]), 0);
  return rows;
}

/** The mail page link of a thread message. */
export function messageUrl(node: Pick<ThreadNode, 'account_id' | 'mailbox' | 'uid'>): string {
  const p = new URLSearchParams({ account_id: node.account_id, mailbox: node.mailbox, uid: String(node.uid) });
  return `/mail/?${p}`;
}

/** The thread dialog's ref for an event's threads button: the first mail the event came from. */
export function threadOfEmail(email: { account_id: string; mailbox: string; uid: number }, event: { uid: string }): ThreadRef {
  return { account_id: email.account_id, mailbox: email.mailbox, uid: email.uid, current: false, event_uid: event.uid };
}
