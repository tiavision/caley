// WebCrypto KEK derivation: PRF bytes → HKDF → AES-256-GCM key.
// The resulting key is non-extractable — it only ever lives in the Web Crypto
// key store and is used immediately for encrypt/decrypt.

export function randomBytes(n: number): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(n));
}

export function b64uEncode(buf: ArrayBuffer | Uint8Array): string {
  const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  let s = '';
  bytes.forEach(b => s += String.fromCharCode(b));
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
}

export function b64uDecode(s: string): Uint8Array {
  const padded = s.replace(/-/g, '+').replace(/_/g, '/');
  const padLen = (4 - padded.length % 4) % 4;
  const b64 = padded + '='.repeat(padLen);
  const bin = atob(b64);
  return Uint8Array.from(bin, c => c.charCodeAt(0));
}

/**
 * Derive a non-extractable AES-256-GCM key from PRF output bytes.
 * prfBytes   - raw PRF output (32 bytes from WebAuthn PRF extension)
 * hkdfSalt   - 32-byte random HKDF salt (stored in vault, not secret)
 */
export async function deriveKek(
  prfBytes: ArrayBuffer,
  hkdfSalt: Uint8Array,
  info = 'caley-device-kek-v1',
): Promise<CryptoKey> {
  const rawKey = await crypto.subtle.importKey('raw', prfBytes, 'HKDF', false, ['deriveKey']);
  return crypto.subtle.deriveKey(
    {
      name: 'HKDF',
      salt: hkdfSalt,
      info: new TextEncoder().encode(info),
      hash: 'SHA-256',
    },
    rawKey,
    { name: 'AES-GCM', length: 256 },
    false,
    ['encrypt', 'decrypt'],
  );
}

export async function importAesKey(raw: Uint8Array): Promise<CryptoKey> {
  return crypto.subtle.importKey('raw', raw, { name: 'AES-GCM' }, false, ['encrypt', 'decrypt']);
}

export async function deriveSharedKek(shared: ArrayBuffer, salt: Uint8Array): Promise<CryptoKey> {
  const rawKey = await crypto.subtle.importKey('raw', shared, 'HKDF', false, ['deriveKey']);
  return crypto.subtle.deriveKey(
    {
      name: 'HKDF',
      salt,
      info: new TextEncoder().encode('caley-vault-wrap-v1'),
      hash: 'SHA-256',
    },
    rawKey,
    { name: 'AES-GCM', length: 256 },
    false,
    ['encrypt', 'decrypt'],
  );
}

export async function aesEncrypt(kek: CryptoKey, iv: Uint8Array, plaintext: Uint8Array): Promise<ArrayBuffer> {
  return crypto.subtle.encrypt({ name: 'AES-GCM', iv }, kek, plaintext);
}

export async function aesDecrypt(kek: CryptoKey, iv: Uint8Array, ciphertext: Uint8Array): Promise<ArrayBuffer> {
  return crypto.subtle.decrypt({ name: 'AES-GCM', iv }, kek, ciphertext);
}
