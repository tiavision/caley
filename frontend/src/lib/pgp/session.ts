// In-memory session holder for the unlocked private key.
// Cleared on logout, explicit lock, inactivity (5-minute timer), and tab
// hide/unload. JS cannot zero memory — we drop references only.

import type * as openpgp from 'openpgp';

interface Session {
  privateKey: openpgp.PrivateKey;
  inactivityTimer: ReturnType<typeof setTimeout>;
}

let session: Session | null = null;
const INACTIVITY_MS = 5 * 60 * 1000;

function clearTimer() {
  if (session) clearTimeout(session.inactivityTimer);
}

function resetTimer() {
  if (!session) return;
  clearTimer();
  session.inactivityTimer = setTimeout(lock, INACTIVITY_MS);
}

export function setKey(key: openpgp.PrivateKey) {
  clearTimer();
  session = {
    privateKey: key,
    inactivityTimer: setTimeout(lock, INACTIVITY_MS),
  };
}

export function getKey(): openpgp.PrivateKey | null {
  if (!session) return null;
  resetTimer();
  return session.privateKey;
}

export function lock() {
  clearTimer();
  session = null;
}

export function isUnlocked(): boolean {
  return session !== null;
}

// Wire to visibility / unload events (fire once per module load).
if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) lock();
  });
}
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', lock);
}
