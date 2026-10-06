//! Regression tests for FINDING-013: a save that renamed its temp file over
//! the vault path itself would replace a symlinked file with a regular file,
//! and the real target would stop receiving edits. A symlinked note is
//! written through its link; `.cairn/settings.json` is a special case, see
//! below.
//!
//! Run: cargo test -p cairn-core --test adv_verify_fs_06

// Symlinks need Developer Mode or admin rights on Windows.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn open(p: &std::path::Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap())).unwrap()
}

/// By design: config files follow a symlink only when it
/// stays inside the vault. A received vault can link `.cairn/settings.json`
/// to any file of the user's, so a link out of the vault is replaced by a
/// plain file and the file outside is never touched (notes still write
/// through their links).
#[test]
fn fs06_symlinked_settings_json_is_replaced_on_write_config() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("settings.json"), "{\"theme\":\"dark\"}").unwrap();
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join(".cairn")).unwrap();
    symlink(outside.path().join("settings.json"), d.path().join(".cairn/settings.json")).unwrap();
    let v = open(d.path());
    assert_eq!(v.read_config("settings.json").unwrap().as_deref(), Some("{\"theme\":\"dark\"}"));
    v.write_config("settings.json", "{\"theme\":\"light\"}").unwrap();
    let is_link = fs::symlink_metadata(d.path().join(".cairn/settings.json")).unwrap().file_type().is_symlink();
    let target = fs::read_to_string(outside.path().join("settings.json")).unwrap();
    assert!(
        !is_link && target == "{\"theme\":\"dark\"}",
        "settings write reached outside the vault: is_link={is_link} target={target:?}"
    );
    assert_eq!(v.read_config("settings.json").unwrap().as_deref(), Some("{\"theme\":\"light\"}"));
}

/// A symlinked note: every save reaches the real target, and the vault path
/// reads the same text.
#[test]
fn fs06_edit_survives_in_vault_and_reaches_target() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("target.md"), "v0").unwrap();
    let d = tempfile::tempdir().unwrap();
    symlink(outside.path().join("target.md"), d.path().join("link.md")).unwrap();
    let v = open(d.path());
    let n = v.read_note("link.md").unwrap();
    v.write_note("link.md", "v1", Some(&n.hash)).unwrap();
    let n = v.read_note("link.md").unwrap();
    v.write_note("link.md", "v2", Some(&n.hash)).unwrap();
    // The edit is not lost: the vault path holds it.
    assert_eq!(fs::read_to_string(d.path().join("link.md")).unwrap(), "v2");
    // And the real file got both saves.
    assert_eq!(fs::read_to_string(outside.path().join("target.md")).unwrap(), "v2", "target diverged from the vault copy");
}
