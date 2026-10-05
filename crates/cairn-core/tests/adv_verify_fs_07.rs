//! Reproduction for FINDING-050.
//!
//! A save must work for every legal target name (NAME_MAX = 255). A temp
//! name derived from the target, such as `.<file name>.cairn-tmp-<pid>`, is
//! 1 + 11 + digits(pid) bytes longer than the target name, so any target
//! name longer than `255 - 12 - digits(pid)` bytes could not be written.
//! `StdFs` uses a short temp name of its own (`create_temp`), and these
//! tests save names at and just over that limit.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_fs_07 -- --include-ignored

use std::fs;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        fs::write(d.path().join(p), c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

/// The largest target name that still fits once the temp prefix/suffix is added.
fn max_writable_len() -> usize {
    255 - 1 - ".cairn-tmp-".len() - std::process::id().to_string().len()
}

/// Control: exactly at the computed limit the save works.
#[test]
fn name_at_the_temp_name_limit_saves() {
    let n = max_writable_len();
    let name = format!("{}.md", "a".repeat(n - 3));
    assert_eq!(name.len(), n);
    let (_d, v) = setup(&[(&name, "x")]);
    let note = v.read_note(&name).unwrap();
    v.write_note(&name, "edited", Some(&note.hash)).expect("save at the limit must work");
}

/// One byte over the limit: the target name is legal (it was created with
/// std::fs::write) and readable, and Cairn must be able to save it.
#[test]
fn name_one_byte_over_the_temp_name_limit_saves() {
    let n = max_writable_len() + 1;
    let name = format!("{}.md", "a".repeat(n - 3));
    let (d, v) = setup(&[(&name, "x")]);
    assert!(d.path().join(&name).exists(), "the target name itself is legal");
    let note = v.read_note(&name).unwrap();
    let r = v.write_note(&name, "edited", Some(&note.hash));
    assert!(r.is_ok(), "len {n}: save failed: {:?}", r.err().map(|e| e.to_string().replace(&"a".repeat(n - 3), "a*")));
}

/// Reachable entirely inside Cairn: renaming a note to a long CJK title
/// works (rename does not use a temp file), and every later save of that
/// note must work too.
#[test]
fn rename_to_long_cjk_title_then_save() {
    let (_d, v) = setup(&[("draft.md", "draft")]);
    let title: String = "日本語のとても長いノートのタイトル".chars().cycle().take(80).collect();
    let name = format!("{title}.md");
    assert_eq!(name.len(), 243);
    v.rename("draft.md", &name).expect("rename to a legal 243-byte name works");
    let note = v.read_note(&name).unwrap();
    let r = v.write_note(&name, "draft, edited after rename", Some(&note.hash));
    assert!(r.is_ok(), "save after rename failed: {:?}", r.err().map(|e| e.to_string()));
}
