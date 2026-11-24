// What the mail reader shows of a message read by sinter-mail's mail.mime (src/actions/mail.ts body()): its parts in
// reading order (bodies never joined), the badges its summary allows, and what a reply may take from it.
// Pure: no openpgp, no DOM (the page tests import it through the actions' types only).
import type { Address, Message as MimeMessage, Part, Protection } from '@tiavision/sinter-mail';

/** One block of the reader's body. */
export interface ReaderPart {
  id: string;
  kind: 'html' | 'text' | 'encrypted' | 'forwarded';
  /** html: the part's HTML, for email.parse */
  html: string;
  /** text: the part's text; forwarded: the forwarded message's header line */
  text: string;
  /** html: the part holds the sender's copy of protected headers for old clients (email.parse's legacyDisplay) */
  legacyDisplay: boolean;
  /** decrypted from a ciphertext: email.parse's `encrypted` (no remote image without the reader's consent) */
  encrypted: boolean;
  /** decrypted from a part the reader chose to open inside the message: shown apart, never as the message's own */
  opened: boolean;
  /** the part's own signature mark, when the message as a whole is not signed by its sender: a badge name, or '' */
  mark: string;
  /** encrypted: why it shows nothing yet ('pending', 'failed', 'not-root', 'over-limit'), and the step that opens it */
  reason: string;
  step: string;
  /** nesting: 1 inside a forwarded message, 2 inside one forwarded in it, … */
  depth: number;
}

/** `Name <address>`, or what there is; an ambiguous address (null) shows its name only. */
export function addressText(a: Address): string {
  if (a.address && a.name && a.name !== a.address) return `${a.name} <${a.address}>`;
  return a.address ?? a.name ?? '';
}

/** A part's own mark (badge name) for its signature status, when the message is not signed as a whole. */
export function partMark(p: Protection, signedAll: boolean): string {
  if (signedAll || p.signed === null || p.signed === 'pending') return '';
  switch (p.signed) {
    case 'good': return 'signed_part';
    case 'bad': return 'verification_failed';
    case 'no-key': return 'unverified';
    case 'expired': return 'signature_expired';
    case 'revoked': return 'signature_revoked';
    default: return 'unsupported_alg';
  }
}

const part = (p: Part, depth: number, signedAll: boolean): ReaderPart => ({
  id: p.id, kind: 'text', html: '', text: '', legacyDisplay: false, encrypted: p.protection.encrypted,
  opened: !!p.protection.opened, mark: partMark(p.protection, signedAll), reason: '', step: '', depth,
});

/** The message's parts as the reader shows them, a forwarded message's after a line naming it. */
export function readerParts(msg: MimeMessage, signedAll = msg.summary.signed === 'all', depth = 0): ReaderPart[] {
  const out: ReaderPart[] = [];
  for (const p of msg.parts) {
    if (p.type === 'html') out.push({ ...part(p, depth, signedAll), kind: 'html', html: p.html, legacyDisplay: p.legacyDisplay });
    else if (p.type === 'text' || p.type === 'report') out.push({ ...part(p, depth, signedAll), text: p.text });
    else if (p.type === 'encrypted') out.push({ ...part(p, depth, signedAll), kind: 'encrypted', reason: p.reason, step: p.step });
    else {
      const h = p.message.headers;
      const line = [h.from.map(addressText).join(', '), h.subject ?? ''].filter(Boolean).join(' · ');
      out.push({ ...part(p, depth, signedAll), kind: 'forwarded', text: line });
      out.push(...readerParts(p.message, signedAll, depth + 1));
    }
  }
  return out;
}

/**
 * The message-level badges: "encrypted" and "verified" only when the summary says the whole message is (a part that
 * is gets its own mark instead), and a warning when the sender shown is not the one the cryptography proves.
 */
export function messageBadges(msg: MimeMessage): string[] {
  const s = msg.summary;
  const badges: string[] = [];
  if (s.encrypted === 'all') badges.push('encrypted');
  if (s.signed === 'all') badges.push('verified');
  if (s.from === 'differs') badges.push('from_differs');
  else if (s.from === 'unknown' && msg.steps.every((x) => x.state !== 'pending')) badges.push('from_unknown');
  if (msg.steps.some((x) => x.kind === 'decrypt' && x.state === 'failed')) badges.push('decrypt_failed');
  return badges;
}

/** Whether a reply may quote the message: nothing was decrypted, or a good signature binds the sender shown. */
export function quotable(msg: MimeMessage): boolean {
  return msg.summary.encrypted === 'none' || msg.summary.from === 'matches';
}

/** The protected subject, when the payload carried one. */
export function protectedSubject(msg: MimeMessage): string {
  return msg.sources.subject === 'protected' ? msg.headers.subject ?? '' : '';
}

/** Whom a reply goes to, when protected headers decide it (else null: the server's From and Reply-To). */
export function protectedReply(msg: MimeMessage): string[] | null {
  return msg.sources.from === 'protected' || msg.sources.replyTo === 'protected' ? msg.reply.map(addressText) : null;
}
