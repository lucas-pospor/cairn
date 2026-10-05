use serde::Serialize;

pub type Result<T> = std::result::Result<T, CoreError>;

/// Errors reported to callers (and, serialized, to the UI).
#[derive(Debug, thiserror::Error, Serialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum CoreError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("invalid name: {0:?}")]
    InvalidName(String),
    /// The file changed on disk since the caller last read it.
    #[error("file changed on disk: {0}")]
    Conflict(String),
    #[error("not a markdown note: {0}")]
    NotANote(String),
    #[error("cannot move a folder into itself: {0}")]
    MoveIntoSelf(String),
    #[error("io error: {0}")]
    Io(String),
}

impl CoreError {
    /// An I/O error on `path` (vault-relative; "" is the vault folder), worded
    /// for the user: the kinds people can act on get a plain sentence, the
    /// rest the OS text without its "(os error N)" code.
    pub fn io(path: &str, e: std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let what = if path.is_empty() { "the vault folder".to_string() } else { format!("\"{path}\"") };
        let what_cap = if path.is_empty() { "The vault folder".to_string() } else { what.clone() };
        match e.kind() {
            K::NotFound => CoreError::NotFound(path.to_string()),
            K::AlreadyExists => CoreError::AlreadyExists(path.to_string()),
            K::PermissionDenied => CoreError::Io(format!("No permission to access {what}.")),
            K::NotADirectory => CoreError::Io(format!("{what_cap} is a file, not a folder.")),
            K::IsADirectory => CoreError::Io(format!("{what_cap} is a folder, not a file.")),
            K::ReadOnlyFilesystem => CoreError::Io(format!("{what_cap} is on a read-only drive.")),
            K::StorageFull | K::QuotaExceeded => CoreError::Io("The disk is full.".into()),
            _ => CoreError::Io(format!("{what_cap}: {}", os_text(&e))),
        }
    }
}

/// The OS's description of `e` without the "(os error N)" on the end.
pub fn os_text(e: &std::io::Error) -> String {
    let text = e.to_string();
    match e.raw_os_error() {
        Some(code) => text.strip_suffix(&format!(" (os error {code})")).unwrap_or(&text).to_string(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    #[test]
    fn io_errors_are_plain_words() {
        let denied = CoreError::io("Locked/Locked note.md", Error::from_raw_os_error(13));
        assert_eq!(denied, CoreError::Io("No permission to access \"Locked/Locked note.md\".".into()));
        assert_eq!(CoreError::io("", Error::from_raw_os_error(13)), CoreError::Io("No permission to access the vault folder.".into()));
        assert_eq!(CoreError::io("a.md/b", ErrorKind::NotADirectory.into()), CoreError::Io("\"a.md/b\" is a file, not a folder.".into()));
        // Kinds without their own sentence keep the OS text, minus the code.
        let other = CoreError::io("x.md", Error::from_raw_os_error(24));
        assert!(matches!(&other, CoreError::Io(m) if m.starts_with("\"x.md\": ") && !m.contains("os error")), "{other:?}");
        assert_eq!(CoreError::io("x.md", ErrorKind::NotFound.into()), CoreError::NotFound("x.md".into()));
    }
}
