// `pgpVault` actions: the synced OpenPGP private-key vault of the settings page (partials/pgp-settings.html).
// Each one loads the steps (src/lib/pgp/vault-steps.ts, with openpgp) on first use and runs one there, so the
// settings page renders without openpgp; a failed load rejects with pgp_loadError (src/lib/pgp/load.ts).
import { lazyModule } from '../lib/pgp/load.ts';

export type { VaultDeviceView, VaultKeyView, VaultView } from '../lib/pgp/vault-steps.ts';
import type { VaultView } from '../lib/pgp/vault-steps.ts';

const steps = lazyModule(() => import('../lib/pgp/vault-steps.ts'));

export async function status(): Promise<VaultView> { return (await steps()).status(); }
export async function unlock(credId: string): Promise<VaultView> { return (await steps()).unlock(credId); }
export async function lock(): Promise<VaultView> { return (await steps()).lock(); }
export async function reset(): Promise<VaultView> { return (await steps()).reset(); }
export async function readKey(armored: string, passphrase: string): Promise<{ fingerprint: string }> { return (await steps()).readKey(armored, passphrase); }
export async function generateKey(name: string, email: string): Promise<{ fingerprint: string }> { return (await steps()).generateKey(name, email); }
export async function forgetKey(): Promise<void> { (await steps()).forgetKey(); }
export async function store(label: string): Promise<VaultView> { return (await steps()).store(label); }
export async function addIdentity(fingerprint: string, name: string, email: string, comment: string): Promise<VaultView> { return (await steps()).addIdentity(fingerprint, name, email, comment); }
export async function updatePublic(fingerprint: string, armored: string): Promise<VaultView> { return (await steps()).updatePublic(fingerprint, armored); }
export async function exportPublic(fingerprint: string): Promise<void> { (await steps()).exportPublic(fingerprint); }
export async function register(label: string): Promise<VaultView> { return (await steps()).register(label); }
export async function approve(credId: string): Promise<VaultView> { return (await steps()).approve(credId); }
export async function rotate(): Promise<VaultView> { return (await steps()).rotate(); }
export async function removeKey(fingerprint: string): Promise<VaultView> { return (await steps()).removeKey(fingerprint); }
export async function removeDevice(credId: string): Promise<VaultView> { return (await steps()).removeDevice(credId); }
