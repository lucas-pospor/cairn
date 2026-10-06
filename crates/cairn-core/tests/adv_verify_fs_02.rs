//! Regression test for FINDING-011: a watcher hint for a folder whose name is
//! NFD on disk must not remove the whole folder from the index (the app's
//! tree would show it briefly after open, then it would disappear).
//!
//!   cargo test -p cairn-core --test adv_verify_fs_02

use std::fs;
use std::sync::Arc;

use cairn_core::{Change, StdFs, TrashMode, Vault};

#[test]
fn fs02_watcher_hint_keeps_nfd_folder() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("Re\u{301}sume\u{301}")).unwrap();
    fs::write(d.path().join("Re\u{301}sume\u{301}/inside.md"), "in folder").unwrap();
    let fsys = StdFs::new(d.path(), TrashMode::Vault).unwrap();
    let hint = fsys
        .to_vault_path(&d.path().canonicalize().unwrap().join("Re\u{301}sume\u{301}"))
        .unwrap();
    let v = Vault::open(Arc::new(fsys)).unwrap();
    let before: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    let changes = v.rescan_paths(&[hint]).unwrap();
    let after: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    assert!(
        !changes.iter().any(|c| matches!(c, Change::Deleted { .. })),
        "entries before {before:?}; hint produced {changes:?}; entries after {after:?}"
    );
}
