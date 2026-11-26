import { describe, expect, it } from 'vitest';
import type { ThreadNode } from './MailTypes.ts';
import { threadRows } from './thread.ts';

const node = (uid: number, message_id: string, parent: string | null, from: string, sent_at: number): ThreadNode => ({
  uid, account_id: 'a', mailbox: 'INBOX', message_id, parent_message_id: parent, from, subject: 'Hi', sent_at, is_read: true,
});

describe('threadRows', () => {
  it('shows two messages sharing a Message-ID once each and marks only the open one current', () => {
    const real = node(1, '<m@x>', null, 'Alice <alice@x>', 100);
    const forged = node(2, '<m@x>', null, 'Mallory <alice@x.evil>', 50);
    const reply = node(3, '<r@x>', '<m@x>', 'Bob <bob@x>', 200);
    const rows = threadRows([forged, real, reply], [], { account_id: 'a', mailbox: 'INBOX', uid: 1, current: true });
    const messages = rows.filter((r) => r.kind === 'message');
    expect(messages.map((r) => r.node?.uid)).toEqual([2, 3, 1]);
    expect(new Set(messages.map((r) => r.key)).size).toBe(3);
    expect(messages.filter((r) => r.current).map((r) => r.node?.uid)).toEqual([1]);
  });
});
