CREATE TABLE IF NOT EXISTS pgp_vaults (
    user_id             TEXT PRIMARY KEY REFERENCES user_settings(user_id) ON DELETE CASCADE,
    revision            INTEGER NOT NULL DEFAULT 1,
    manifest_iv         TEXT NOT NULL,
    encrypted_manifest  TEXT NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS pgp_private_keys (
    user_id     TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL,
    iv          TEXT NOT NULL,
    ciphertext  TEXT NOT NULL,
    PRIMARY KEY (user_id, fingerprint)
);

CREATE TABLE IF NOT EXISTS pgp_devices (
    cred_id                 TEXT PRIMARY KEY REFERENCES webauthn_credentials(cred_id) ON DELETE CASCADE,
    user_id                 TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    status                  TEXT NOT NULL CHECK (status IN ('pending', 'active')),
    public_jwk              TEXT NOT NULL,
    encrypted_private_jwk   TEXT NOT NULL,
    private_iv              TEXT NOT NULL,
    prf_salt                TEXT NOT NULL,
    hkdf_salt               TEXT NOT NULL,
    pairing_nonce           TEXT NOT NULL,
    created_at              INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS pgp_devices_user ON pgp_devices(user_id, status);

CREATE TABLE IF NOT EXISTS pgp_vault_wraps (
    user_id               TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    cred_id               TEXT NOT NULL REFERENCES pgp_devices(cred_id) ON DELETE CASCADE,
    ephemeral_public_jwk  TEXT NOT NULL,
    hkdf_salt             TEXT NOT NULL,
    iv                    TEXT NOT NULL,
    ciphertext            TEXT NOT NULL,
    PRIMARY KEY (user_id, cred_id)
);
