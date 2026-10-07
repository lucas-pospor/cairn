//! Cairn core: everything that is not UI.
//!
//! The vault folder is the source of truth. [`Vault`] keeps an in-memory
//! [`index::Index`] built from the files and updates it on every operation
//! and on every rescan.

// The Windows Recycle Bin's size limit, worked out apart from Windows so
// that its tests run everywhere.
#[cfg(any(all(windows, feature = "system-trash"), test))]
mod bin_size;
pub mod error;
pub mod fs;
pub mod index;
pub mod parse;
pub mod path;
#[cfg(all(windows, feature = "system-trash"))]
mod recycle_bin;
pub mod search;
pub mod vault;

pub use error::{CoreError, Result};
pub use fs::{EntryKind, FileStat, StdFs, TrashMode, VaultFs};
pub use vault::{Change, NoteContent, Vault, WriteResult};
