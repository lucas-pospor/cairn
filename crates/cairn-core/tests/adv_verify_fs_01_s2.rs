//! Reproduction for FINDING-003: scope of the case-only
//! rename overwrite on a case-sensitive file system (Linux, Android app
//! storage).
//!
//! * The precondition (two names that differ only in case) can be made from
//!   inside Cairn itself; no outside tool is needed.
//! * Only a FILE target is at risk. A case-twin folder that has contents
//!   makes rename(2) fail (ENOTEMPTY), so whole folders are safe; an empty
//!   twin folder is replaced, which loses nothing.
//! * An overwritten file would not be moved anywhere: `StdFs::rename` never
//!   goes through `remove()`, so neither the vault `.trash` nor the system
//!   trash would see it. The rename must therefore be refused.
//!
//!   cargo test -p cairn-core --test adv_verify_fs_01_s2 -- --include-ignored --nocapture

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{CoreError, StdFs, TrashMode, Vault};

fn vault_with(files: &[(&str, &str)], mode: TrashMode) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), mode).unwrap())).unwrap();
    (d, v)
}

/// Every file under `dir`, hidden ones included, with its content.
fn all_files(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            all_files(&p, root, out);
        } else {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            out.push((rel, fs::read_to_string(&p).unwrap_or_default()));
        }
    }
}

/// Precondition is reachable in-app: with `Note.md` present, creating a new
/// note and renaming it to `note.md` is allowed on Linux. Documents that a
/// user can get into the state FINDING-003 needs without any outside tool.
/// Creating the twin directly (`note.md`, or a `projects` folder next to
/// `Projects`) is refused (FINDING-053).
#[test]
fn precondition_case_twins_can_be_made_inside_cairn() {
    let (d, v) = vault_with(&[("Note.md", "upper\n")], TrashMode::Vault);
    assert!(matches!(v.create_note("note.md", ""), Err(CoreError::AlreadyExists(_))));
    v.create_note("Untitled.md", "fresh\n").unwrap();
    v.rename("Untitled.md", "note.md").unwrap();
    assert_eq!(fs::read_to_string(d.path().join("Note.md")).unwrap(), "upper\n");
    assert_eq!(fs::read_to_string(d.path().join("note.md")).unwrap(), "fresh\n");
    v.create_folder("Projects").unwrap();
    assert!(matches!(v.create_folder("projects"), Err(CoreError::AlreadyExists(_))));
    assert!(!d.path().join("projects").exists());
}

/// Control (passes): a case-only FOLDER rename onto a non-empty twin folder
/// fails in rename(2) and both folders keep their notes.
#[test]
fn folder_case_rename_onto_nonempty_twin_fails_safely() {
    let (d, v) = vault_with(&[("Projects/a.md", "A\n"), ("projects/b.md", "B\n")], TrashMode::Vault);
    let r = v.rename("Projects", "projects");
    println!("folder rename onto non-empty twin -> {r:?}");
    assert!(r.is_err(), "rename returned {r:?}");
    assert_eq!(fs::read_to_string(d.path().join("Projects/a.md")).unwrap(), "A\n");
    assert_eq!(fs::read_to_string(d.path().join("projects/b.md")).unwrap(), "B\n");
}

/// Control (passes): a file renamed onto a twin that is a folder also fails
/// (EISDIR); the folder and its notes survive.
#[test]
fn file_case_rename_onto_twin_folder_fails_safely() {
    let (d, v) = vault_with(&[("Todo", "file\n"), ("todo/x.md", "X\n")], TrashMode::Vault);
    let r = v.rename("Todo", "todo");
    println!("file rename onto twin folder -> {r:?}");
    assert!(r.is_err(), "rename returned {r:?}");
    assert_eq!(fs::read_to_string(d.path().join("todo/x.md")).unwrap(), "X\n");
    assert_eq!(fs::read_to_string(d.path().join("Todo")).unwrap(), "file\n");
}

/// The defect itself, checked against every trash mode the app uses
/// (desktop opens vaults with TrashMode::System): `a.md -> A.md` must be
/// refused, and the old `A.md` text must still be under the vault. An
/// overwrite would leave it nowhere, `.trash` included, behind a plain
/// success.
#[test]
fn overwritten_target_is_not_trashed_in_any_mode() {
    let mut bad = Vec::new();
    for mode in [TrashMode::System, TrashMode::Vault, TrashMode::Permanent] {
        let (d, v) = vault_with(&[("a.md", "lower\n"), ("A.md", "UPPER precious\n")], mode);
        let r = v.rename("a.md", "A.md");
        let mut files = Vec::new();
        all_files(d.path(), d.path(), &mut files);
        let survivors: Vec<_> = files.iter().filter(|(_, c)| c.contains("precious")).map(|(p, _)| p.clone()).collect();
        println!("{mode:?}: rename -> {r:?}; files now {files:?}");
        if !matches!(r, Err(CoreError::AlreadyExists(_))) || survivors.is_empty() {
            bad.push(format!("{mode:?}: rename ok={} and \"UPPER precious\" survives in {survivors:?}", r.is_ok()));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
