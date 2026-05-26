// WebAuthn enrollment and unlock, calling the server start/finish endpoints.
// PRF output is extracted from getClientExtensionResults() and never sent
// to the server.

import { b64uEncode, b64uDecode } from './kek.ts';

// Convert a WebAuthn credential for JSON serialisation (base64url-encode binary fields).
function credentialToJson(cred: PublicKeyCredential): unknown {
  const response = cred.response as AuthenticatorAttestationResponse | AuthenticatorAssertionResponse;

  if ('attestationObject' in response) {
    // Registration response
    const att = response as AuthenticatorAttestationResponse;
    return {
      id: cred.id,
      rawId: b64uEncode(cred.rawId),
      type: cred.type,
      response: {
        attestationObject: b64uEncode(att.attestationObject),
        clientDataJSON: b64uEncode(att.clientDataJSON),
      },
    };
  } else {
    // Authentication response
    const ass = response as AuthenticatorAssertionResponse;
    return {
      id: cred.id,
      rawId: b64uEncode(cred.rawId),
      type: cred.type,
      response: {
        authenticatorData: b64uEncode(ass.authenticatorData),
        clientDataJSON: b64uEncode(ass.clientDataJSON),
        signature: b64uEncode(ass.signature),
        userHandle: ass.userHandle ? b64uEncode(ass.userHandle) : null,
      },
    };
  }
}

// Decode a server challenge JSON into the form expected by navigator.credentials.
function decodeCreationOptions(json: Record<string, unknown>): PublicKeyCredentialCreationOptions {
  const pk = json.publicKey as Record<string, unknown>;
  return {
    ...pk,
    challenge: b64uDecode(pk.challenge as string),
    user: {
      ...(pk.user as Record<string, unknown>),
      id: b64uDecode(((pk.user as Record<string, unknown>).id) as string),
    },
    excludeCredentials: ((pk.excludeCredentials as unknown[]) ?? []).map((c: unknown) => {
      const cr = c as Record<string, unknown>;
      return { ...cr, id: b64uDecode(cr.id as string) };
    }),
  } as PublicKeyCredentialCreationOptions;
}

function decodeRequestOptions(json: Record<string, unknown>): PublicKeyCredentialRequestOptions {
  const pk = json.publicKey as Record<string, unknown>;
  const extensions = pk.extensions as Record<string, unknown> | undefined;
  const prfEval = (extensions?.prf as Record<string, unknown> | undefined)?.eval as Record<string, unknown> | undefined;

  return {
    ...pk,
    challenge: b64uDecode(pk.challenge as string),
    allowCredentials: ((pk.allowCredentials as unknown[]) ?? []).map((c: unknown) => {
      const cr = c as Record<string, unknown>;
      return { ...cr, id: b64uDecode(cr.id as string) };
    }),
    extensions: extensions ? {
      ...extensions,
      prf: prfEval
        ? { eval: { first: b64uDecode(prfEval.first as string) } }
        : (extensions.prf ?? undefined),
    } : undefined,
  } as PublicKeyCredentialRequestOptions;
}

export interface EnrollResult {
  credentialId: string;
  prfOutput: ArrayBuffer;   // raw PRF bytes — used once to wrap the vault, then discarded
  hasPrf: boolean;
}

/**
 * Enrollment = register + immediate authenticate (to capture first PRF output).
 * deviceLabel is stored in the server's credential registry.
 */
export async function enroll(deviceLabel: string): Promise<EnrollResult> {
  // --- Register ---
  const startRes = await fetch('/api/webauthn/register/start', { method: 'POST' });
  if (!startRes.ok) throw new Error('register/start failed: ' + await startRes.text());
  const startJson = await startRes.json() as Record<string, unknown>;

  const creationOptions = decodeCreationOptions(startJson);
  const credential = await navigator.credentials.create({ publicKey: creationOptions });
  if (!credential) throw new Error('WebAuthn create() returned null');

  const finishRes = await fetch('/api/webauthn/register/finish', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ credential: credentialToJson(credential as PublicKeyCredential), device_label: deviceLabel, has_prf: false }),
  });
  if (!finishRes.ok) throw new Error('register/finish failed: ' + await finishRes.text());
  const { cred_id } = await finishRes.json() as { cred_id: string };

  // --- Authenticate immediately to capture PRF output ---
  const prfSalt = new Uint8Array(32);
  crypto.getRandomValues(prfSalt);
  const prfSaltB64 = b64uEncode(prfSalt);

  const authStartRes = await fetch('/api/webauthn/authenticate/start', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ prf_salt: prfSaltB64 }),
  });
  if (!authStartRes.ok) throw new Error('authenticate/start failed: ' + await authStartRes.text());
  const authJson = await authStartRes.json() as Record<string, unknown>;

  const requestOptions = decodeRequestOptions(authJson);
  const authCred = await navigator.credentials.get({ publicKey: requestOptions });
  if (!authCred) throw new Error('WebAuthn get() returned null');

  const extResults = (authCred as PublicKeyCredential).getClientExtensionResults() as Record<string, unknown>;
  const prf = extResults?.prf as Record<string, unknown> | undefined;
  const prfOutput = (prf?.results as Record<string, unknown>)?.first as ArrayBuffer | undefined;

  const hasPrf = !!prfOutput;
  if (!hasPrf) throw new Error('prf_not_supported');

  const authFinishRes = await fetch('/api/webauthn/authenticate/finish', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(credentialToJson(authCred as PublicKeyCredential)),
  });
  if (!authFinishRes.ok) throw new Error('authenticate/finish failed: ' + await authFinishRes.text());

  // Update has_prf on the server (fire-and-forget)
  fetch('/api/webauthn/register/finish', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ credential: credentialToJson(credential as PublicKeyCredential), device_label: deviceLabel, has_prf: true }),
  }).catch(() => {});

  return { credentialId: cred_id, prfOutput, hasPrf: true };
}

export interface CredentialInfo {
  cred_id: string;
  device_label: string | null;
  has_prf: boolean;
}

export async function listCredentials(): Promise<CredentialInfo[]> {
  const res = await fetch('/api/webauthn/credentials');
  if (!res.ok) throw new Error('list credentials failed: ' + await res.text());
  return res.json() as Promise<CredentialInfo[]>;
}

export async function deleteCredential(credId: string): Promise<void> {
  const res = await fetch('/api/webauthn/credential', {
    method: 'DELETE',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ cred_id: credId }),
  });
  if (!res.ok) throw new Error('delete credential failed: ' + await res.text());
}

/**
 * Resume enrollment — authenticate only (no registration).
 * Used when a previous enrollment registered the credential but failed before writing the vault.
 * prfSalt is a fresh random value (will be stored in the new vault).
 */
export async function resumeEnroll(prfSaltB64: string): Promise<EnrollResult> {
  const authStartRes = await fetch('/api/webauthn/authenticate/start', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ prf_salt: prfSaltB64 }),
  });
  if (!authStartRes.ok) throw new Error('authenticate/start failed: ' + await authStartRes.text());
  const authJson = await authStartRes.json() as Record<string, unknown>;

  const requestOptions = decodeRequestOptions(authJson);
  const authCred = await navigator.credentials.get({ publicKey: requestOptions });
  if (!authCred) throw new Error('WebAuthn get() returned null');

  const extResults = (authCred as PublicKeyCredential).getClientExtensionResults() as Record<string, unknown>;
  const prf = extResults?.prf as Record<string, unknown> | undefined;
  const prfOutput = (prf?.results as Record<string, unknown>)?.first as ArrayBuffer | undefined;

  if (!prfOutput) throw new Error('prf_not_supported');

  const finishRes = await fetch('/api/webauthn/authenticate/finish', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(credentialToJson(authCred as PublicKeyCredential)),
  });
  if (!finishRes.ok) throw new Error('authenticate/finish failed: ' + await finishRes.text());

  return { credentialId: (authCred as PublicKeyCredential).id, prfOutput, hasPrf: true };
}

export interface UnlockResult {
  prfOutput: ArrayBuffer;
  prfSaltUsed: string;   // base64url, so the caller can verify it matches vault.prfSalt
}

/**
 * Unlock = authenticate with the stored PRF salt.
 * Returns the raw PRF bytes; the caller derives the KEK and decrypts the vault.
 */
export async function unlock(prfSaltB64: string): Promise<UnlockResult> {
  const authStartRes = await fetch('/api/webauthn/authenticate/start', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ prf_salt: prfSaltB64 }),
  });
  if (!authStartRes.ok) throw new Error('authenticate/start failed: ' + await authStartRes.text());
  const authJson = await authStartRes.json() as Record<string, unknown>;

  const requestOptions = decodeRequestOptions(authJson);
  const authCred = await navigator.credentials.get({ publicKey: requestOptions });
  if (!authCred) throw new Error('WebAuthn get() returned null');

  const extResults = (authCred as PublicKeyCredential).getClientExtensionResults() as Record<string, unknown>;
  const prf = extResults?.prf as Record<string, unknown> | undefined;
  const prfOutput = (prf?.results as Record<string, unknown>)?.first as ArrayBuffer | undefined;
  if (!prfOutput) throw new Error('prf_not_supported');

  const finishRes = await fetch('/api/webauthn/authenticate/finish', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(credentialToJson(authCred as PublicKeyCredential)),
  });
  if (!finishRes.ok) throw new Error('authenticate/finish failed: ' + await finishRes.text());

  return { prfOutput, prfSaltUsed: prfSaltB64 };
}
