//! Reproduction for FINDING-049: scope of the watcher impact. A hinted
//! rescan must not fail when one of its hints is (or contains) an unreadable
//! folder, and neither must a full rescan (manual rescan, sync, watcher
//! overflow). The unreadable folder is skipped and what the index had in it
//! is kept.
//!
//!   cargo test -p cairn-core --test adv_verify_fs_03 -- --include-ignored --nocapture

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn chmod(p: &Path, mode: u32) {
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

fn vault_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

/// Control: an unrelated hinted rescan is not affected.
#[test]
fn fs03_hinted_rescan_of_other_paths_still_works() {
    let (d, v) = vault_with(&[("ok.md", "fine"), ("private/x.md", "x")]);
    chmod(&d.path().join("private"), 0o000);
    fs::write(d.path().join("new.md"), "made in another editor").unwrap();
    let hinted = v.rescan_paths(&["new.md".into()]);
    chmod(&d.path().join("private"), 0o755);
    let changes = hinted.expect("hinted rescan of new.md only");
    println!("changes: {changes:?}");
    assert!(v.index().note("new.md").is_some());
}

/// A batch that also mentions the unreadable folder (the chmod itself
/// produces such an event) keeps every other change in that batch, and a
/// full rescan succeeds.
#[test]
fn fs03_batch_with_unreadable_folder_loses_other_changes() {
    let (d, v) = vault_with(&[("ok.md", "fine"), ("private/x.md", "x")]);
    chmod(&d.path().join("private"), 0o000);
    fs::write(d.path().join("new.md"), "made in another editor").unwrap();
    let batch = v.rescan_paths(&["new.md".into(), "private".into()]);
    let full = v.rescan();
    chmod(&d.path().join("private"), 0o755);
    println!("batch: {:?}\nfull: {:?}", batch.as_ref().err(), full.as_ref().err());
    assert!(batch.is_ok() && full.is_ok());
    assert!(v.index().note("new.md").is_some());
    // Skipping must not drop what is already indexed under the folder.
    assert!(v.index().entry("private/x.md").is_some());
}

/// Moving a folder the index has not seen yet, with a subfolder whose
/// entries cannot be opened (mode 644), and listing a config folder with
/// one: the unreadable subfolder is skipped, not an error. An error here
/// would come after the move happened on disk, and the index would miss the
/// moved folder until the next rescan.
#[test]
fn fs03_unreadable_subfolder_does_not_fail_a_move_or_a_config_listing() {
    let (d, v) = vault_with(&[("n.md", "n")]);
    for p in ["inbox/ok.md", "inbox/locked/x.md", ".cairn/plugins/p.js", ".cairn/plugins/locked/y.js"] {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, "x").unwrap();
    }
    chmod(&d.path().join("inbox/locked"), 0o644);
    chmod(&d.path().join(".cairn/plugins/locked"), 0o644);
    let moved = v.rename("inbox", "moved");
    let config = v.list_config("plugins");
    chmod(&d.path().join("moved/locked"), 0o755);
    chmod(&d.path().join(".cairn/plugins/locked"), 0o755);
    moved.expect("move");
    let mut paths: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    paths.sort();
    assert_eq!(paths, vec!["moved", "moved/locked", "moved/ok.md", "n.md"]);
    assert_eq!(v.unreadable_folders(), vec!["moved/locked".to_string()]);
    assert_eq!(config.expect("config listing"), vec!["p.js".to_string()]);
}

/// A hint inside the unreadable folder (a file made there before the chmod,
/// seen late) cannot be looked up: the batch's other changes still apply,
/// and the index keeps what it had there.
#[test]
fn fs03_hint_inside_an_unreadable_folder_does_not_fail_the_batch() {
    let (d, v) = vault_with(&[("ok.md", "fine"), ("private/x.md", "x")]);
    chmod(&d.path().join("private"), 0o000);
    fs::write(d.path().join("new.md"), "made in another editor").unwrap();
    let batch = v.rescan_paths(&["private/x.md".into(), "private/y.md".into(), "new.md".into()]);
    chmod(&d.path().join("private"), 0o755);
    batch.expect("batch");
    assert!(v.index().note("new.md").is_some());
    assert!(v.index().entry("private/x.md").is_some());
}

/// A symlink whose target this user may not reach (in a folder without
/// permission) is unknown, not deleted: the index keeps it, and a link in
/// the vault folder itself does not stop the vault from opening.
#[test]
fn fs03_a_link_that_cannot_be_followed_is_kept() {
    let outside = tempfile::tempdir().unwrap();
    let locked = outside.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("o.md"), "outside").unwrap();
    let (d, v) = vault_with(&[("n.md", "n")]);
    fs::create_dir(d.path().join("refs")).unwrap();
    std::os::unix::fs::symlink(locked.join("o.md"), d.path().join("refs/o.md")).unwrap();
    std::os::unix::fs::symlink(locked.join("o.md"), d.path().join("top.md")).unwrap();
    v.rescan().unwrap();
    chmod(&locked, 0o000);
    let rescan = v.rescan();
    let reopened = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap()));
    chmod(&locked, 0o755);
    rescan.expect("rescan");
    let mut paths: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    paths.sort();
    assert_eq!(paths, vec!["n.md", "refs", "refs/o.md", "top.md"]);
    assert_eq!(v.unreadable_folders(), vec!["refs/o.md".to_string(), "top.md".to_string()]);
    assert_eq!(reopened.expect("open").unreadable_folders(), vec!["refs/o.md".to_string(), "top.md".to_string()]);
}
