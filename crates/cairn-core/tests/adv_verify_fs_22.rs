//! Reproduction for FINDING-053: creating a note or folder whose name
//! differs only in case from an existing sibling must be refused (docs/PLAN.md
//! section 6: refuse new case twins on case-insensitive file systems).
//!
//! Run: cargo test -p cairn-core --test adv_verify_fs_22 -- --ignored

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{CoreError, StdFs, TrashMode, Vault};

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// The UI path for a broken link `[[projects/idea]]` when the folder on disk
/// is `Projects/`: the link does not resolve (no note named "idea"), so the app
/// calls create_note("projects/idea.md"), which must not silently create a
/// second folder `projects/` next to `Projects/`.
#[test]
fn fs22_create_note_in_case_variant_parent_creates_second_folder() {
    let (d, v) = setup(&[("Projects/plan.md", "plan")]);
    let r = v.create_note("projects/idea.md", "");
    let top = names(d.path());
    assert!(
        r.is_err(),
        "create_note returned {:?}; vault root now holds {top:?}",
        r.map(|w| w.entry.path)
    );
}

/// Creating Note.md next to note.md would also silently re-target every
/// existing link to the old note: link resolution is case-insensitive at every
/// rank, so [[note]] would tie between the two and the alphabetical tie-break
/// would pick the new, empty "Note.md" (uppercase sorts first).
#[test]
fn fs22_case_duplicate_hijacks_existing_links() {
    let (_d, v) = setup(&[("note.md", "the real note"), ("other.md", "see [[note]]")]);
    assert_eq!(v.resolve("note", "other.md").as_deref(), Some("note.md"));
    let r = v.create_note("Note.md", "");
    if matches!(r, Err(CoreError::AlreadyExists(_))) {
        return; // the duplicate is refused
    }
    assert_eq!(
        v.resolve("note", "other.md").as_deref(),
        Some("note.md"),
        "after create_note returned {:?}, [[note]] in other.md no longer points at note.md",
        r.map(|w| w.entry.path)
    );
}

/// The "New folder" prompt passes the typed name straight to create_folder.
#[test]
fn fs22_create_folder_case_variant() {
    let (d, v) = setup(&[("Folder/a.md", "a")]);
    let r = v.create_folder("folder");
    assert!(
        matches!(r, Err(CoreError::AlreadyExists(_))),
        "create_folder returned {r:?}; root holds {:?}",
        names(d.path())
    );
}
