CREATE TABLE IF NOT EXISTS webauthn_credentials (
    cred_id     TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    passkey     TEXT NOT NULL,
    sign_count  INTEGER NOT NULL DEFAULT 0,
    device_label TEXT,
    has_prf     INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS webauthn_credentials_user ON webauthn_credentials(user_id);

CREATE TABLE IF NOT EXISTS pgp_public_keys (
    user_id     TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL,
    email       TEXT NOT NULL DEFAULT '',
    armored     TEXT NOT NULL,
    is_own      INTEGER NOT NULL DEFAULT 0,
    source      TEXT NOT NULL DEFAULT 'imported',
    trust       TEXT NOT NULL DEFAULT 'unverified',
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (user_id, fingerprint)
);
CREATE INDEX IF NOT EXISTS pgp_public_keys_user_email ON pgp_public_keys(user_id, email);

ALTER TABLE cached_messages ADD COLUMN is_encrypted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cached_messages ADD COLUMN is_signed    INTEGER NOT NULL DEFAULT 0;
