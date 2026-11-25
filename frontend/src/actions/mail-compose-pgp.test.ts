// @vitest-environment node
// Real openpgp needs node (see src/lib/pgp/sign-verify.test.ts).
// Which keys compose encrypts to (mail.composeKeys, mail.send): for each recipient only the trusted keys stored for
// exactly that address, never a key that merely carries the address in a user ID, never a key found in mail that the
// user has not accepted; the sender's own key by its user IDs, as before.
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import * as openpgp from 'openpgp';
import type { CallContext } from '@tiaforge-ui/sinter';
import { m } from '@tiaforge-ui/sinter/messages';
import { composeKeys, send } from './mail.ts';
import { canEncrypt, composeDraft, waitingKeys, type ComposeDraft, type Identity } from '../lib/mail.ts';
import type { StoredPublicKey } from '../lib/pgp/public-keys.ts';

const keys = (...emails: string[]) => openpgp.generateKey({ userIDs: emails.map((email) => ({ name: email.split('@')[0], email })), format: 'object' });
const record = (key: openpgp.PublicKey, email: string, extra: Partial<StoredPublicKey> = {}): StoredPublicKey => ({
  fingerprint: key.getFingerprint().toUpperCase(), email, armored: key.armor(), is_own: false, source: 'imported', trust: 'trusted', ...extra,
});
const identities: Identity[] = [{ identity_id: 'i1', account_id: 'work', name: 'Me', email: 'me@example.com', is_default: true, sort_order: 0 } as Identity];

let stored: StoredPublicKey[] = [];
beforeEach(() => {
  stored = [];
  vi.stubGlobal('fetch', async (url: string) => (url === '/api/pgp/keys' ? Response.json(stored) : new Response('', { status: 404 })));
});
afterEach(() => vi.unstubAllGlobals());

function draft(to: string): ComposeDraft {
  return { ...composeDraft('new', null, identities), to, subject: 'Hi', body: 'Secret', encrypt: true };
}

/** Sends the draft; answers the key IDs its ciphertext is encrypted to. */
async function sentTo(d: ComposeDraft): Promise<string[]> {
  let raw = '';
  const ctx = {
    signal: new AbortController().signal,
    request: async (_url: string, init: { body: FormData }) => { raw = await (init.body.get('raw_mime') as Blob).text(); return null; },
  } as unknown as CallContext;
  await send(d, identities, false, ctx);
  const armored = /-----BEGIN PGP MESSAGE-----[\s\S]*?-----END PGP MESSAGE-----/.exec(raw)![0];
  const message = await openpgp.readMessage({ armoredMessage: armored });
  return message.getEncryptionKeyIDs().map((id) => id.toHex()).sort();
}
const encryptionKeyId = async (key: openpgp.PublicKey) => (await key.getEncryptionKey()).getKeyID().toHex();

test('a recipient\'s keys are the trusted keys stored for that address, not a key that only names it in a user ID', async () => {
  const me = await keys('me@example.com');
  const bob = await keys('bob@example.com');
  const mallory = await keys('mallory@evil.example', 'bob@example.com');   // claims Bob's address too
  stored = [
    record(me.publicKey, 'me@example.com', { is_own: true }),
    record(bob.publicKey, 'bob@example.com'),
    record(mallory.publicKey, 'mallory@evil.example'),
  ];
  const ck = await composeKeys();
  expect(ck.keys.find((k) => !k.is_own && k.fingerprint === record(mallory.publicKey, '').fingerprint)!.emails).toEqual(['mallory@evil.example']);
  expect(await sentTo(draft('Bob <bob@example.com>'))).toEqual([await encryptionKeyId(bob.publicKey), await encryptionKeyId(me.publicKey)].sort());
});

test('a key found in mail that is not accepted is never encrypted to: compose lists it to accept, and sending encrypted is refused', async () => {
  const me = await keys('me@example.com');
  const carol = await keys('carol@example.com');
  stored = [
    record(me.publicKey, 'me@example.com', { is_own: true }),
    record(carol.publicKey, 'carol@example.com', { source: 'harvested_autocrypt', trust: 'unverified' }),
  ];
  const ck = await composeKeys();
  const d = draft('carol@example.com');
  expect(canEncrypt(d, identities, ck)).toBe(false);
  expect(waitingKeys(d, ck)).toEqual([{ fingerprint: record(carol.publicKey, '').fingerprint, email: 'carol@example.com', userIds: ['carol <carol@example.com>'], change: false }]);
  await expect(sentTo(d)).rejects.toThrow(m.pgp_composeMissingKeys({ emails: 'carol@example.com' }));
});

test('a key found in mail that differs from the trusted one is a key change: the trusted key is still the one encrypted to', async () => {
  const me = await keys('me@example.com');
  const bob = await keys('bob@example.com');
  const other = await keys('bob@example.com');
  stored = [
    record(me.publicKey, 'me@example.com', { is_own: true }),
    record(bob.publicKey, 'bob@example.com'),
    record(other.publicKey, 'bob@example.com', { source: 'harvested_autocrypt', trust: 'unverified' }),
  ];
  const ck = await composeKeys();
  const d = draft('bob@example.com');
  expect(waitingKeys(d, ck).map((k) => k.change)).toEqual([true]);
  expect(await sentTo(d)).toEqual([await encryptionKeyId(bob.publicKey), await encryptionKeyId(me.publicKey)].sort());
});
