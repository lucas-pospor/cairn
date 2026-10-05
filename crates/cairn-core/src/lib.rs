//! Cairn core: everything that is not UI.
//!
//! The vault folder is the source of truth. [`Vault`] keeps an in-memory
//! [`index::Index`] built from the files and updates it on every operation
//! and on every rescan.

pub mod error;
pub mod fs;
pub mod index;
pub mod parse;
pub mod path;
pub mod search;
pub mod vault;

pub use error::{CoreError, Result};
pub use fs::{EntryKind, FileStat, StdFs, TrashMode, VaultFs};
pub use vault::{Change, NoteContent, Vault, WriteResult};
