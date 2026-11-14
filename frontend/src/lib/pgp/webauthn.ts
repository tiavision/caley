// WebAuthn enrollment and unlock, calling the server start/finish endpoints.
// PRF output is extracted from getClientExtensionResults() and never sent
// to the server.

import { b64uEncode, b64uDecode } from './kek.ts';
import { apiFetch } from '../api.ts';

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
function decodeCreationOptions(
  json: Record<string, unknown>,
  prfSalt?: Uint8Array,
): PublicKeyCredentialCreationOptions {
  const pk = json.publicKey as Record<string, unknown>;
  const extensions = pk.extensions as Record<string, unknown> | undefined;
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
    extensions: prfSalt ? {
      ...extensions,
      prf: { eval: { first: prfSalt } },
    } : extensions,
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
  prfSalt: string;         // base64url PRF input used to produce prfOutput
  prfOutput: ArrayBuffer;   // raw PRF bytes — used once to wrap the vault, then discarded
  hasPrf: boolean;
}

function getPrfOutput(credential: PublicKeyCredential): ArrayBuffer | undefined {
  const extResults = credential.getClientExtensionResults() as Record<string, unknown>;
  const prf = extResults?.prf as Record<string, unknown> | undefined;
  const first = (prf?.results as Record<string, unknown> | undefined)?.first;
  return first instanceof ArrayBuffer ? first : undefined;
}

/**
 * Enrollment requests a PRF output during registration. Authenticators that
 * support PRF but do not return its value at creation (such as Samsung Pass)
 * are authenticated once afterward to obtain it.
 */
export async function enroll(deviceLabel: string): Promise<EnrollResult> {
  const prfSalt = new Uint8Array(32);
  crypto.getRandomValues(prfSalt);
  const prfSaltB64 = b64uEncode(prfSalt);

  // --- Register ---
  const startRes = await apiFetch('/api/webauthn/register/start', { method: 'POST' });
  if (!startRes.ok) throw new Error('register/start failed: ' + await startRes.text());
  const startJson = await startRes.json() as Record<string, unknown>;

  const creationOptions = decodeCreationOptions(startJson, prfSalt);
  const credential = await navigator.credentials.create({ publicKey: creationOptions });
  if (!credential) throw new Error('WebAuthn create() returned null');
  const registrationCredential = credential as PublicKeyCredential;
  const creationPrfOutput = getPrfOutput(registrationCredential);

  const finishRes = await apiFetch('/api/webauthn/register/finish', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ credential: credentialToJson(registrationCredential), device_label: deviceLabel, has_prf: false }),
  });
  if (!finishRes.ok) throw new Error('register/finish failed: ' + await finishRes.text());
  const { cred_id } = await finishRes.json() as { cred_id: string };

  if (creationPrfOutput) {
    return { credentialId: cred_id, prfSalt: prfSaltB64, prfOutput: creationPrfOutput, hasPrf: true };
  }

  // Some authenticators expose PRF support at creation but evaluate it only
  // during authentication.
  const authStartRes = await apiFetch('/api/webauthn/authenticate/start', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ prf_salt: prfSaltB64, credential_id: cred_id }),
  });
  if (!authStartRes.ok) throw new Error('authenticate/start failed: ' + await authStartRes.text());
  const authJson = await authStartRes.json() as Record<string, unknown>;

  const requestOptions = decodeRequestOptions(authJson);
  const authCred = await navigator.credentials.get({ publicKey: requestOptions });
  if (!authCred) throw new Error('WebAuthn get() returned null');

  const prfOutput = getPrfOutput(authCred as PublicKeyCredential);
  if (!prfOutput) throw new Error('prf_not_supported');

  const authFinishRes = await apiFetch('/api/webauthn/authenticate/finish', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(credentialToJson(authCred as PublicKeyCredential)),
  });
  if (!authFinishRes.ok) throw new Error('authenticate/finish failed: ' + await authFinishRes.text());
  const authenticated = await authFinishRes.json() as { cred_id: string };
  if (authenticated.cred_id !== cred_id) throw new Error('wrong credential selected');

  return { credentialId: cred_id, prfSalt: prfSaltB64, prfOutput, hasPrf: true };
}

export interface CredentialInfo {
  cred_id: string;
  device_label: string | null;
  has_prf: boolean;
}

export async function listCredentials(): Promise<CredentialInfo[]> {
  const res = await apiFetch('/api/webauthn/credentials');
  if (!res.ok) throw new Error('list credentials failed: ' + await res.text());
  return res.json() as Promise<CredentialInfo[]>;
}

export interface UnlockResult {
  prfOutput: ArrayBuffer;
  prfSaltUsed: string;   // base64url, so the caller can verify it matches vault.prfSalt
}

/**
 * Unlock = authenticate with the stored PRF salt.
 * Returns the raw PRF bytes; the caller derives the KEK and decrypts the vault.
 */
export async function unlock(prfSaltB64: string, credentialId?: string): Promise<UnlockResult> {
  const authStartRes = await apiFetch('/api/webauthn/authenticate/start', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ prf_salt: prfSaltB64, credential_id: credentialId }),
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

  const finishRes = await apiFetch('/api/webauthn/authenticate/finish', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(credentialToJson(authCred as PublicKeyCredential)),
  });
  if (!finishRes.ok) throw new Error('authenticate/finish failed: ' + await finishRes.text());
  const authenticated = await finishRes.json() as { cred_id: string };
  if (credentialId && authenticated.cred_id !== credentialId) throw new Error('wrong credential selected');

  return { prfOutput, prfSaltUsed: prfSaltB64 };
}
