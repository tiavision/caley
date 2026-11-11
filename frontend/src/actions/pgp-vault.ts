// `pgpVault` actions: the synced OpenPGP private-key vault of the settings page (src/components/pgp-settings.html).
// The steps are src/lib/pgp/vault-steps.ts; openpgp, which they use, is a lazy package (sinter({ lazy })): the settings
// page renders without it, and the first step that calls it loads it.
export type { VaultDeviceView, VaultKeyView, VaultView } from '../lib/pgp/vault-steps.ts';
export {
  status, unlock, lock, reset, readKey, generateKey, forgetKey, store, addIdentity, updatePublic, exportPublic, register,
  approve, rotate, removeKey, removeDevice,
} from '../lib/pgp/vault-steps.ts';
