//! Wire format between Cairn clients and the sync server (JSON over HTTP).
//!
//! The server stores opaque encrypted blobs. It sees vault ids, random file
//! ids, revision numbers, sizes and times, but never paths or contents.

use serde::{Deserialize, Serialize};

pub const API_PREFIX: &str = "/v1";

/// Key material a new device needs to unlock a vault. Everything here is
/// safe to store on the server: the vault key is wrapped with a key derived
/// from the passphrase, which never leaves the devices.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyEnvelope {
    /// Argon2id salt (base64).
    pub salt: String,
    /// Argon2id parameters.
    pub kdf: KdfParams,
    /// The 32-byte vault key encrypted with the derived key (base64,
    /// nonce followed by ciphertext).
    pub wrapped_key: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultInfo {
    pub vault: String,
    pub keys: KeyEnvelope,
    /// Highest revision number in the vault.
    pub head_seq: u64,
}

/// One file's current head, as returned by the changes feed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteHead {
    pub file_id: String,
    pub seq: u64,
    pub parent_seq: Option<u64>,
    pub device: String,
    pub deleted: bool,
    /// Encrypted payload (base64). Present for deletions too, so the path
    /// of a deleted file is known to clients.
    pub blob: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangesResponse {
    pub heads: Vec<RemoteHead>,
    /// Pass as `since` to continue.
    pub cursor: u64,
    pub more: bool,
}

/// Upload a new revision. Accepted only if the file's current head is
/// `parent_seq` (None = the file must not exist yet).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PutRevision {
    pub parent_seq: Option<u64>,
    pub device: String,
    pub deleted: bool,
    pub blob: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PutResult {
    pub seq: u64,
}

/// Body of a 409 response: someone else moved the head first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Conflict {
    pub current_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    pub seq: u64,
    pub created: i64,
    pub device: String,
    pub deleted: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionBlob {
    pub seq: u64,
    pub file_id: String,
    pub deleted: bool,
    pub blob: String,
}

/// Valid vault and file ids: 1 to 64 chars of `[A-Za-z0-9_-]`.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}
