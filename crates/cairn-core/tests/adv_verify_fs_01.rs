//! Reproduction for FINDING-003: a case-only rename (or a move into a
//! folder whose name differs only in case) onto an existing, different file
//! on a case-sensitive file system must be refused, not replace that file.
//! These tests also check that nothing is lost: they search the whole vault
//! (including `.trash` and every other hidden folder) for the target's
//! content after the rename.
//!
//!   cargo test -p cairn-core --test adv_verify_fs_01 -- --ignored

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{CoreError, StdFs, TrashMode, Vault};

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

/// Every file under `dir` (hidden ones too) whose content contains `needle`.
fn find_content(dir: &Path, root: &Path, needle: &str, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if p.is_dir() {
            find_content(&p, root, needle, out);
        } else if fs::read_to_string(&p).is_ok_and(|s| s.contains(needle)) {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
        }
    }
}

fn report(d: &tempfile::TempDir, needle: &str) -> String {
    let mut hits = Vec::new();
    find_content(d.path(), d.path(), needle, &mut hits);
    let trash = d.path().join(".trash");
    format!(
        "files still holding {needle:?}: {hits:?}; .trash exists: {}",
        trash.exists()
    )
}

#[test]
fn case_only_rename_onto_other_file_is_refused_and_nothing_is_lost() {
    let (d, v) = vault_with(&[("a.md", "lower\n"), ("A.md", "UPPER precious\n")]);
    let ino_upper = fs::metadata(d.path().join("A.md")).unwrap().ino();
    let r = v.rename("a.md", "A.md");
    let now_ino = fs::metadata(d.path().join("A.md")).map(|m| m.ino()).ok();
    assert!(
        matches!(r, Err(CoreError::AlreadyExists(_))),
        "rename returned {r:?}; A.md inode was {ino_upper}, now {now_ino:?}; {}",
        report(&d, "UPPER precious")
    );
    assert_eq!(fs::read_to_string(d.path().join("A.md")).unwrap(), "UPPER precious\n");
    assert_eq!(fs::read_to_string(d.path().join("a.md")).unwrap(), "lower\n");
}

#[test]
fn move_into_case_twin_folder_is_refused_and_nothing_is_lost() {
    let (d, v) = vault_with(&[("Projects/todo.md", "upper folder\n"), ("projects/todo.md", "lower folder PRECIOUS\n")]);
    let r = v.rename("Projects/todo.md", "projects/todo.md");
    assert!(
        matches!(r, Err(CoreError::AlreadyExists(_))),
        "rename returned {r:?}; {}",
        report(&d, "PRECIOUS")
    );
}

/// The guard is meant for renaming one entry to a new case of its own name.
/// That keeps working (control: this one passes and must keep passing).
#[test]
fn control_true_case_only_rename_of_a_single_file_still_works() {
    let (d, v) = vault_with(&[("note.md", "x\n")]);
    v.rename("note.md", "Note.md").unwrap();
    assert_eq!(fs::read_to_string(d.path().join("Note.md")).unwrap(), "x\n");
    assert!(!d.path().join("note.md").exists());
}

/// Control: a plain (non case-only) rename onto an existing file is refused,
/// so the overwrite is specific to the case-only shortcut.
#[test]
fn control_plain_rename_onto_existing_file_is_refused() {
    let (d, v) = vault_with(&[("a.md", "a\n"), ("b.md", "b\n")]);
    assert!(matches!(v.rename("a.md", "b.md"), Err(CoreError::AlreadyExists(_))));
    assert_eq!(fs::read_to_string(d.path().join("b.md")).unwrap(), "b\n");
}
