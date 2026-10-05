//! Cairn sync.
//!
//! * [`protocol`]: JSON types shared by client and server.
//! * [`crypto`]: passphrase-derived keys and per-revision encryption.
//! * [`engine`] (feature `client`): compares a vault with its last synced
//!   state and with the server, merges, and uploads.
//!
//! See `docs/PLAN.md` for the conflict rules.

pub mod protocol;

#[cfg(feature = "client")]
pub mod crypto;
#[cfg(feature = "client")]
pub mod engine;
#[cfg(feature = "client")]
pub mod transport;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("wrong passphrase for this vault")]
    WrongPassphrase,
    #[error("encryption error: {0}")]
    Crypto(String),
    #[error("server error: {0}")]
    Server(String),
    #[error("cannot reach the server: {0}")]
    Network(String),
    /// One upload failed on its own (too large for the server or this
    /// client, or stalled or cut off while the server still answers); other
    /// uploads can still work.
    #[error("upload failed: {0}")]
    Upload(String),
    #[error("the server rejected the token")]
    Unauthorized,
    #[error("vault {0:?} does not exist on the server")]
    NoSuchVault(String),
    /// The server has fewer changes than this device has seen: it was reset
    /// or restored from an older backup.
    #[error("the server seems to have been reset, so nothing was synced. Turn sync off in Settings > Sync and connect again")]
    ServerReset,
    /// The server no longer has the vault this device synced with: it was
    /// reset, or the vault was removed there (or another Cairn server now
    /// answers at the server's address).
    #[error("the server no longer has this vault, so nothing was synced. If the server was reset, turn sync off in Settings > Sync and connect again")]
    VaultGone,
    /// The vault folder has no files, but this device has synced some: it
    /// may have been moved or renamed, or its storage is not mounted.
    /// Pushing that would delete every note on every device.
    #[error("the vault folder looks empty or missing, so nothing was synced. If you deleted every note on purpose, add a note and sync again")]
    VaultEmpty,
    #[error("local error: {0}")]
    Local(String),
    /// A local file changed on disk while sync was writing it
    /// (`CoreError::Conflict`): the engine starts the round again. Kept apart
    /// from `Local`, whose messages can quote any file name.
    #[error("local error: file changed on disk: {0}")]
    Changed(String),
    #[error("sync is not set up for this vault")]
    NotConfigured,
}

#[cfg(feature = "client")]
impl From<cairn_core::CoreError> for SyncError {
    fn from(e: cairn_core::CoreError) -> Self {
        match e {
            // Worded for the user already (`CoreError::io`), so without the
            // "io error: " of its Display.
            cairn_core::CoreError::Io(m) => SyncError::Local(m),
            cairn_core::CoreError::Conflict(p) => SyncError::Changed(p),
            e => SyncError::Local(e.to_string()),
        }
    }
}

impl From<std::io::Error> for SyncError {
    fn from(e: std::io::Error) -> Self {
        SyncError::Local(e.to_string())
    }
}

/// Three-way merge of a text file changed in two places since `base`, the
/// way sync merges a note edited on two devices. `None` when the changes
/// touch the same or neighbouring lines, or when a side changed too many
/// lines to merge quickly (the limit sync uses).
#[cfg(feature = "client")]
pub fn merge_text(base: &str, ours: &str, theirs: &str) -> Option<String> {
    engine::merge_text(base, ours, theirs)
}
