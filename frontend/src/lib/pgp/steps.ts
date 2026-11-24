// The OpenPGP steps sinter-mail's mail.mime asks for (src/actions/mail.ts body()): decrypting the message's root and
// verifying signatures, with openpgp.js. mail.mime reads the structure and decides what each result proves; this
// module holds the keys and decides whom to trust.
// CSP constraint: no 'unsafe-eval', no wasm, so Argon2 S2K is reported as unsupported.

import { SinterError } from '@tiaforge-ui/sinter';
import * as mail from '@tiavision/sinter-mail';
import type { Decrypted, Message, MimeOptions, Ref, Step, Verdict } from '@tiavision/sinter-mail';
import * as openpgp from 'openpgp';
import type { StoredPublicKey } from './public-keys.ts';

/** A public key caley knows, with the addresses it is trusted for: the ones it was stored under. */
export interface KnownKey { key: openpgp.PublicKey; addresses: string[] }

/** The trusted stored keys, one per fingerprint, each with every address it was stored for. */
export async function knownKeys(records: StoredPublicKey[]): Promise<KnownKey[]> {
  const byFingerprint = new Map<string, { armored: string; addresses: Set<string> }>();
  for (const r of records) {
    if (r.trust !== 'trusted') continue;
    const fp = r.fingerprint.toUpperCase();
    const entry = byFingerprint.get(fp) ?? { armored: r.armored, addresses: new Set<string>() };
    if (r.email) entry.addresses.add(r.email.trim().toLowerCase());
    byFingerprint.set(fp, entry);
  }
  const out: KnownKey[] = [];
  for (const { armored, addresses } of byFingerprint.values()) {
    try {
      out.push({ key: await openpgp.readKey({ armoredKey: armored }), addresses: [...addresses] });
    } catch (e) {
      if (e instanceof SinterError) throw e;
      // a corrupt stored record cannot verify anything
    }
  }
  return out;
}

export type Results = Required<Pick<MimeOptions, 'decrypted' | 'verdicts' | 'open'>> & Pick<MimeOptions, 'holes'>;

/**
 * `raw` read by mail.mime with every step settled into `r` (new objects each time: mail.mime caches by identity;
 * sinter-mail is a lazy package, so each of its calls is awaited):
 * decrypted with `me` (none: 'locked'), verified with the keys `known` loads (only once a step needs them).
 */
export async function settle(raw: Uint8Array, r: Results, me: openpgp.PrivateKey[], known: () => Promise<KnownKey[]>): Promise<Message> {
  let msg = await mail.mime(raw, r);
  let keys: KnownKey[] | null = null;
  // each round settles every pending step; a message has at most 16
  // a step whose bytes were cut out (a cut message, `holes`) takes no result: the caller reads the whole message
  const pending = (m: Message) => m.steps.filter((x) => x.state === 'pending' && !x.incomplete);
  for (let round = 0; round <= 16 && pending(msg).length; round++) {
    keys ??= await known();
    for (const s of pending(msg)) {
      if (s.kind === 'decrypt') r.decrypted = { ...r.decrypted, [s.id]: await decryptStep(await mail.bytes(raw, s.ciphertext, r), me, keys) };
      else r.verdicts = { ...r.verdicts, [s.id]: await verifyStep(s, (ref) => mail.bytes(raw, ref, r), keys) };
    }
    msg = await mail.mime(raw, r);
  }
  return msg;
}

function hasArgon2(err: unknown): boolean {
  return err instanceof Error && /argon2/i.test(err.message);
}

/**
 * A decrypt step's result. `integrity` is true only for an encrypted message whose integrity-protected packet
 * (SEIPD with MDC, or AEAD) decrypted completely: `allowUnauthenticatedMessages: false`, binary, not streamed.
 */
export async function decryptStep(ciphertext: Uint8Array, me: openpgp.PrivateKey[], known: KnownKey[]): Promise<Decrypted> {
  if (me.length === 0) return { status: 'locked', integrity: false };
  try {
    const text = new TextDecoder().decode(ciphertext);
    const message = text.includes('-----BEGIN PGP MESSAGE-----')
      ? await openpgp.readMessage({ armoredMessage: text })
      : await openpgp.readMessage({ binaryMessage: ciphertext });
    // a signed or literal message is not encrypted: never 'ok'
    const ids = message.getEncryptionKeyIDs();
    if (ids.length === 0) return { status: 'failed', integrity: false };
    const mine = ids.some((id) => id.toHex() === '0000000000000000' || me.some((k) => k.getKeys(id).length > 0));
    if (!mine) return { status: 'no-key', integrity: false };
    const { data, signatures } = await openpgp.decrypt({
      message, decryptionKeys: me, verificationKeys: known.map((k) => k.key), format: 'binary',
      config: { allowUnauthenticatedMessages: false },
    });
    return {
      status: 'ok', plaintext: data as Uint8Array, integrity: true,
      signatures: await Promise.all(signatures.map((s) => verdict(s, known))),
    };
  } catch (e) {
    if (e instanceof SinterError) throw e;   // openpgp could not be loaded: not a message that failed to decrypt
    return { status: hasArgon2(e) ? 'unsupported' : 'failed', integrity: false };
  }
}

/** A verify step's verdicts (a detached PGP/MIME signature, or an inline cleartext-signed block). */
export async function verifyStep(step: Step, get: (ref: Ref) => Uint8Array | Promise<Uint8Array>, known: KnownKey[]): Promise<Verdict[]> {
  const keys = known.map((k) => k.key);
  try {
    if (step.kind === 'verify') {
      const sig = await get(step.signature);
      const text = new TextDecoder().decode(sig);
      const signature = text.includes('-----BEGIN PGP SIGNATURE-----')
        ? await openpgp.readSignature({ armoredSignature: text })
        : await openpgp.readSignature({ binarySignature: sig });
      const message = await openpgp.createMessage({ binary: await get(step.signed) });
      const { signatures } = await openpgp.verify({ message, signature, verificationKeys: keys });
      return await Promise.all(signatures.map((s) => verdict(s, known)));
    }
    if (step.kind === 'verify-cleartext') {
      const cleartextMessage = new TextDecoder(step.charset).decode(await get(step.block));
      const message = await openpgp.readCleartextMessage({ cleartextMessage });
      const { signatures } = await openpgp.verify({ message, verificationKeys: keys });
      return await Promise.all(signatures.map((s) => verdict(s, known)));
    }
  } catch (e) {
    if (e instanceof SinterError) throw e;   // openpgp could not be loaded: not a bad signature
    return [{ status: 'unsupported' }];
  }
  return [];
}

async function verdict(sig: openpgp.DecryptMessageResult['signatures'][number], known: KnownKey[]): Promise<Verdict> {
  const k = known.find((x) => x.key.getKeys(sig.keyID).length > 0);
  if (!k) return { status: 'no-key' };
  const fingerprint = k.key.getFingerprint().toUpperCase();
  try {
    await sig.verified;
  } catch (e) {
    if (e instanceof SinterError) throw e;
    const msg = e instanceof Error ? e.message : '';
    if (/expired/i.test(msg)) return { status: 'expired', fingerprint };
    if (/revoked/i.test(msg)) return { status: 'revoked', fingerprint };
    return { status: 'bad', fingerprint };
  }
  // signer: only the addresses caley trusts this key for, never every user ID the key claims
  return { status: 'good', fingerprint, ...(k.addresses.length ? { signer: k.addresses } : {}) };
}
