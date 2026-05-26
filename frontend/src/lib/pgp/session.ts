// In-memory session holder for unlocked private keys.
// Cleared on logout, explicit lock, inactivity (5-minute timer), and tab
// hide/unload. JS cannot zero memory — we drop references only.

import type * as openpgp from 'openpgp';

interface SessionEntry {
  privateKey: openpgp.PrivateKey;
  inactivityTimer: ReturnType<typeof setTimeout>;
}

const sessions = new Map<string, SessionEntry>();
const INACTIVITY_MS = 5 * 60 * 1000;

function clearEntryTimer(fingerprint: string) {
  const entry = sessions.get(fingerprint);
  if (entry) clearTimeout(entry.inactivityTimer);
}

function resetEntryTimer(fingerprint: string) {
  const entry = sessions.get(fingerprint);
  if (!entry) return;
  clearTimeout(entry.inactivityTimer);
  entry.inactivityTimer = setTimeout(() => lockKey(fingerprint), INACTIVITY_MS);
}

export function setKey(fingerprint: string, key: openpgp.PrivateKey) {
  clearEntryTimer(fingerprint);
  sessions.set(fingerprint, {
    privateKey: key,
    inactivityTimer: setTimeout(() => lockKey(fingerprint), INACTIVITY_MS),
  });
}

export function getKeys(): openpgp.PrivateKey[] {
  const keys: openpgp.PrivateKey[] = [];
  for (const [fp, entry] of sessions) {
    resetEntryTimer(fp);
    keys.push(entry.privateKey);
  }
  return keys;
}

export function getKey(fingerprint: string): openpgp.PrivateKey | null {
  const entry = sessions.get(fingerprint);
  if (!entry) return null;
  resetEntryTimer(fingerprint);
  return entry.privateKey;
}

export function lockKey(fingerprint: string) {
  clearEntryTimer(fingerprint);
  sessions.delete(fingerprint);
}

export function lock() {
  for (const fp of sessions.keys()) clearEntryTimer(fp);
  sessions.clear();
}

export function isUnlocked(): boolean {
  return sessions.size > 0;
}

export function isKeyUnlocked(fingerprint: string): boolean {
  return sessions.has(fingerprint);
}

if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) lock();
  });
}
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', lock);
}
