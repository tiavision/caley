use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hkdf::Hkdf;
use sha2::Sha256;

const HKDF_SALT: [u8; 32] = [
    b'c', b'a', b'l', b'e', b'y', b'-', b'e', b'n', b'c', b'r', b'y', b'p', b't', b'i', b'o', b'n',
    b'-', b'k', b'e', b'y', b'-', b's', b'a', b'l', b't', b'-', b'v', b'1', 0, 0, 0, 0,
];
const HKDF_INFO: &[u8] = b"caley-aes256gcm-key";

pub fn derive_key(passphrase: &str) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(Some(&HKDF_SALT), passphrase.as_bytes());
    let mut key = [0u8; 32];
    hkdf.expand(HKDF_INFO, &mut key)
        .expect("HKDF expand failed");
    key
}

pub fn encrypt(plaintext: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key)?;
    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&combined))
}

pub fn decrypt(encoded: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    let combined = BASE64.decode(encoded)?;
    if combined.len() < 13 {
        anyhow::bail!("ciphertext too short");
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce_bytes: [u8; 12] = nonce_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid nonce length"))?;
    let cipher = Aes256Gcm::new_from_slice(key)?;
    let nonce = Nonce::from(nonce_bytes);
    let plaintext = cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;
    String::from_utf8(plaintext).map_err(|e| anyhow::anyhow!("invalid utf-8: {e}"))
}
