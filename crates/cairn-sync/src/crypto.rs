//! End-to-end encryption.
//!
//! passphrase --Argon2id(salt)--> key-encryption key --unwraps--> vault key
//! vault key --XChaCha20-Poly1305--> each file revision
//!
//! Each blob is `version byte || 24-byte nonce || ciphertext`. The file id is
//! bound as associated data, so the server cannot swap blobs between files.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::protocol::{b64, unb64, KdfParams, KeyEnvelope};
use crate::SyncError;

const BLOB_VERSION: u8 = 1;
const KEY_AAD: &[u8] = b"cairn-vault-key-v1";

/// Defaults: 64 MiB memory, 3 passes. Takes well under a second on phones
/// from the last several years and makes guessing passphrases expensive.
pub const DEFAULT_KDF: KdfParams = KdfParams { m_cost_kib: 64 * 1024, t_cost: 3, p_cost: 1 };

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("OS random number generator unavailable");
    b
}

/// 32-byte symmetric key for one vault.
#[derive(Clone)]
pub struct VaultKey([u8; 32]);

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey(..)")
    }
}

impl Drop for VaultKey {
    fn drop(&mut self) {
        // best effort wipe
        for b in self.0.iter_mut() {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

fn derive_kek(passphrase: &str, salt: &[u8], kdf: KdfParams) -> Result<[u8; 32], SyncError> {
    let params = Params::new(kdf.m_cost_kib, kdf.t_cost, kdf.p_cost, Some(32))
        .map_err(|e| SyncError::Crypto(format!("bad KDF parameters: {e}")))?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| SyncError::Crypto(format!("key derivation failed: {e}")))?;
    Ok(out)
}

impl VaultKey {
    pub fn generate() -> VaultKey {
        VaultKey(random_bytes())
    }

    pub fn from_bytes(b: [u8; 32]) -> VaultKey {
        VaultKey(b)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Wrap this key with a passphrase for storage on the server.
    pub fn wrap(&self, passphrase: &str, kdf: KdfParams) -> Result<KeyEnvelope, SyncError> {
        let salt: [u8; 16] = random_bytes();
        let kek = derive_kek(passphrase, &salt, kdf)?;
        let cipher = XChaCha20Poly1305::new((&kek).into());
        let nonce: [u8; 24] = random_bytes();
        let ct = cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: &self.0, aad: KEY_AAD })
            .map_err(|_| SyncError::Crypto("wrapping the vault key failed".into()))?;
        let mut wrapped = nonce.to_vec();
        wrapped.extend_from_slice(&ct);
        Ok(KeyEnvelope { salt: b64(&salt), kdf, wrapped_key: b64(&wrapped) })
    }

    /// Unwrap with a passphrase. A wrong passphrase fails here, before any
    /// file is touched.
    pub fn unwrap(env: &KeyEnvelope, passphrase: &str) -> Result<VaultKey, SyncError> {
        let salt = unb64(&env.salt).ok_or_else(|| SyncError::Crypto("bad salt".into()))?;
        let wrapped = unb64(&env.wrapped_key).ok_or_else(|| SyncError::Crypto("bad wrapped key".into()))?;
        if wrapped.len() < 24 + 16 {
            return Err(SyncError::Crypto("wrapped key too short".into()));
        }
        let kek = derive_kek(passphrase, &salt, env.kdf)?;
        let cipher = XChaCha20Poly1305::new((&kek).into());
        let pt = cipher
            .decrypt(XNonce::from_slice(&wrapped[..24]), Payload { msg: &wrapped[24..], aad: KEY_AAD })
            .map_err(|_| SyncError::WrongPassphrase)?;
        let key: [u8; 32] = pt.try_into().map_err(|_| SyncError::Crypto("bad key length".into()))?;
        Ok(VaultKey(key))
    }

    pub fn encrypt(&self, file_id: &str, plaintext: &[u8]) -> Vec<u8> {
        let cipher = XChaCha20Poly1305::new((&self.0).into());
        let nonce: [u8; 24] = random_bytes();
        let ct = cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext, aad: file_id.as_bytes() })
            .expect("encryption cannot fail");
        let mut out = Vec::with_capacity(1 + 24 + ct.len());
        out.push(BLOB_VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    pub fn decrypt(&self, file_id: &str, blob: &[u8]) -> Result<Vec<u8>, SyncError> {
        if blob.len() < 1 + 24 + 16 || blob[0] != BLOB_VERSION {
            return Err(SyncError::Crypto(format!("unsupported blob for {file_id}")));
        }
        let cipher = XChaCha20Poly1305::new((&self.0).into());
        cipher
            .decrypt(XNonce::from_slice(&blob[1..25]), Payload { msg: &blob[25..], aad: file_id.as_bytes() })
            .map_err(|_| SyncError::Crypto(format!("cannot decrypt {file_id}: wrong key or tampered data")))
    }
}

/// What is inside an encrypted blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePayload {
    pub path: String,
    pub mtime: i64,
    pub data: Vec<u8>,
}

impl FilePayload {
    /// `u32 path length || path || i64 mtime || data`, little endian.
    pub fn encode(&self) -> Vec<u8> {
        let p = self.path.as_bytes();
        let mut out = Vec::with_capacity(4 + p.len() + 8 + self.data.len());
        out.extend_from_slice(&(p.len() as u32).to_le_bytes());
        out.extend_from_slice(p);
        out.extend_from_slice(&self.mtime.to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(b: &[u8]) -> Result<FilePayload, SyncError> {
        let bad = || SyncError::Crypto("malformed payload".into());
        let n = u32::from_le_bytes(b.get(..4).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let path = std::str::from_utf8(b.get(4..4 + n).ok_or_else(bad)?).map_err(|_| bad())?.to_string();
        let mtime = i64::from_le_bytes(b.get(4 + n..12 + n).ok_or_else(bad)?.try_into().unwrap());
        Ok(FilePayload { path, mtime, data: b[12 + n..].to_vec() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: KdfParams = KdfParams { m_cost_kib: 1024, t_cost: 1, p_cost: 1 };

    #[test]
    fn wrap_unwrap_and_wrong_passphrase() {
        let k = VaultKey::generate();
        let env = k.wrap("correct horse", FAST).unwrap();
        let k2 = VaultKey::unwrap(&env, "correct horse").unwrap();
        assert_eq!(k.as_bytes(), k2.as_bytes());
        assert!(matches!(VaultKey::unwrap(&env, "wrong"), Err(SyncError::WrongPassphrase)));
    }

    #[test]
    fn blobs_are_bound_to_file_ids_and_detect_tampering() {
        let k = VaultKey::generate();
        let p = FilePayload { path: "a/b.md".into(), mtime: 42, data: b"secret note".to_vec() };
        let blob = k.encrypt("f1", &p.encode());
        assert!(!blob.windows(6).any(|w| w == b"secret"));
        assert_eq!(FilePayload::decode(&k.decrypt("f1", &blob).unwrap()).unwrap(), p);
        assert!(k.decrypt("f2", &blob).is_err());
        let mut t = blob.clone();
        *t.last_mut().unwrap() ^= 1;
        assert!(k.decrypt("f1", &t).is_err());
        assert!(VaultKey::generate().decrypt("f1", &blob).is_err());
        // same plaintext encrypts differently each time
        assert_ne!(k.encrypt("f1", &p.encode()), blob);
    }
}
