//! Adversarial tests: file operations through the public
//! cairn-core API: rename, delete, write, permissions, symlinks, trash, path
//! validation of every entry point.
//!
//! Tests named `fsNN_*` are regression tests for fixed findings.
//! Run one with:
//!   cargo test -p cairn-core --test adv_fs_ops -- --exact <name>
//! Run them all with:
//!   cargo test -p cairn-core --test adv_fs_ops

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use cairn_core::{Change, CoreError, StdFs, TrashMode, Vault};

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = open(d.path());
    (d, v)
}

fn open(p: &Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap())).unwrap()
}

fn read(d: &tempfile::TempDir, rel: &str) -> String {
    fs::read_to_string(d.path().join(rel)).unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[cfg(unix)]
fn chmod(p: &Path, mode: u32) {
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

// ---------------------------------------------------------------------------
// FINDING-003: case-only renames never overwrite an existing, different file
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")] // Case twins need a case-sensitive file system.
#[test]
fn fs01_case_only_rename_onto_existing_file_is_refused() {
    // Linux is case-sensitive: a.md and A.md are two different notes.
    let (d, v) = setup(&[("a.md", "lower"), ("A.md", "UPPER precious")]);
    let r = v.rename("a.md", "A.md");
    assert!(matches!(r, Err(CoreError::AlreadyExists(_))), "rename returned {r:?}");
    assert_eq!(read(&d, "A.md"), "UPPER precious", "A.md was overwritten");
}

#[cfg(target_os = "linux")] // Case twins need a case-sensitive file system.
#[test]
fn fs01_move_between_case_differing_folders_is_refused() {
    // Drag "Projects/todo.md" into the (different) folder "projects/".
    let (d, v) = setup(&[("Projects/todo.md", "upper folder todo"), ("projects/todo.md", "lower folder todo PRECIOUS")]);
    let r = v.rename("Projects/todo.md", "projects/todo.md");
    assert!(matches!(r, Err(CoreError::AlreadyExists(_))), "rename returned {r:?}");
    assert_eq!(read(&d, "projects/todo.md"), "lower folder todo PRECIOUS");
}

#[cfg(target_os = "linux")] // Case twins need a case-sensitive file system.
#[test]
fn fs01_non_ascii_case_only_rename_is_refused() {
    let (d, v) = setup(&[("Über.md", "upper umlaut"), ("über.md", "lower umlaut PRECIOUS")]);
    let r = v.rename("Über.md", "über.md");
    assert!(matches!(r, Err(CoreError::AlreadyExists(_))), "rename returned {r:?}");
    assert_eq!(read(&d, "über.md"), "lower umlaut PRECIOUS");
}

#[test]
fn case_only_rename_of_a_single_file_works() {
    let (d, v) = setup(&[("note.md", "x"), ("Dir/n.md", "n")]);
    v.rename("note.md", "Note.md").unwrap();
    assert_eq!(names(d.path()), vec!["Dir", "Note.md"]);
    v.rename("Dir", "dir").unwrap();
    assert_eq!(read(&d, "dir/n.md"), "n");
    assert!(v.index().note("dir/n.md").is_some());
}

#[test]
fn rename_refuses_existing_targets_self_moves_and_bad_paths() {
    let (d, v) = setup(&[("a/x.md", "ax"), ("b/y.md", "by"), ("f.md", "f"), ("g/h.md", "h")]);
    assert!(matches!(v.rename("a", "b"), Err(CoreError::AlreadyExists(_))));
    assert!(matches!(v.rename("f.md", "g"), Err(CoreError::AlreadyExists(_))));
    assert!(matches!(v.rename("g", "f.md"), Err(CoreError::AlreadyExists(_))));
    assert!(matches!(v.rename("f.md", "b/y.md"), Err(CoreError::AlreadyExists(_))));
    assert!(matches!(v.rename("a", "a/b/a"), Err(CoreError::MoveIntoSelf(_))));
    assert!(matches!(v.rename("a", "a/a"), Err(CoreError::MoveIntoSelf(_))));
    assert_eq!(v.rename("f.md", "f.md").unwrap(), vec![]);
    assert_eq!(v.rename("f.md", "f.md/").unwrap(), vec![]);
    assert!(matches!(v.rename("f.md", ".hidden/f.md"), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.rename("f.md", "a/.git/f.md"), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.rename("f.md", ""), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.rename("", "x"), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.rename("f.md", "../f.md"), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.rename("f.md", "bad:name.md"), Err(CoreError::InvalidName(_))));
    // An absolute path is treated as vault-relative, never as an OS path.
    assert!(matches!(v.rename("f.md", "/tmp/f.md"), Err(CoreError::NotFound(_))));
    for (p, c) in [("a/x.md", "ax"), ("b/y.md", "by"), ("f.md", "f"), ("g/h.md", "h")] {
        assert_eq!(read(&d, p), c);
    }
}

// ---------------------------------------------------------------------------
// FINDING-013: saving keeps symlinked / hard-linked notes linked
// ---------------------------------------------------------------------------

#[cfg(unix)] // Symlinks need Developer Mode or admin rights on Windows.
#[test]
fn fs06_saving_symlinked_note_keeps_symlink() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("target.md"), "outside original").unwrap();
    let d = tempfile::tempdir().unwrap();
    symlink(outside.path().join("target.md"), d.path().join("link.md")).unwrap();
    let v = open(d.path());
    let n = v.read_note("link.md").unwrap();
    assert_eq!(n.content, "outside original");
    v.write_note("link.md", "edited via cairn", Some(&n.hash)).unwrap();
    assert!(fs::symlink_metadata(d.path().join("link.md")).unwrap().file_type().is_symlink(), "link.md is no longer a symlink");
    assert_eq!(fs::read_to_string(outside.path().join("target.md")).unwrap(), "edited via cairn", "symlink target was not updated");
}

#[cfg(unix)] // Stable std has no link count on Windows, so StdFs cannot keep hard links there.
#[test]
fn fs06_saving_hard_linked_note_keeps_link() {
    let (d, v) = setup(&[("hard.md", "hard")]);
    fs::hard_link(d.path().join("hard.md"), d.path().join("other-name.md")).unwrap();
    let n = v.read_note("hard.md").unwrap();
    v.write_note("hard.md", "hard edited", Some(&n.hash)).unwrap();
    assert_eq!(fs::metadata(d.path().join("hard.md")).unwrap().nlink(), 2, "hard link was broken");
    assert_eq!(read(&d, "other-name.md"), "hard edited");
}

#[cfg(unix)] // Symlinks need Developer Mode or admin rights on Windows.
#[test]
fn symlinked_folder_delete_only_removes_the_link() {
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(outside.path().join("dir")).unwrap();
    fs::write(outside.path().join("dir/secret.md"), "outside secret").unwrap();
    let d = tempfile::tempdir().unwrap();
    symlink(outside.path().join("dir"), d.path().join("linkdir")).unwrap();
    symlink(d.path().join("nowhere.md"), d.path().join("dangling.md")).unwrap();
    let v = open(d.path());
    let mut p: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    p.sort();
    // Linked folders are followed (by design); dangling links are skipped.
    assert_eq!(p, vec!["linkdir", "linkdir/secret.md"]);
    v.delete("linkdir").unwrap();
    assert_eq!(fs::read_to_string(outside.path().join("dir/secret.md")).unwrap(), "outside secret");
    assert!(fs::symlink_metadata(d.path().join(".trash/linkdir")).unwrap().file_type().is_symlink());
}

// ---------------------------------------------------------------------------
// FINDING-050: long names (the temp file name must fit NAME_MAX too)
// ---------------------------------------------------------------------------

#[test]
fn fs07_note_with_240_byte_name_can_be_created() {
    let (_d, v) = setup(&[]);
    let name = format!("{}.md", "b".repeat(240)); // 243 bytes, legal on ext4/tmpfs (255)
    let r = v.create_note(&name, "x");
    assert!(r.is_ok(), "create_note failed: {:?}", r.err().map(|e| e.to_string().replace(&"b".repeat(240), "b*240")));
}

#[test]
fn fs07_external_long_note_can_be_saved() {
    // 80 CJK characters = 240 bytes: a plausible Japanese/Chinese title.
    let title: String = "日本語のとても長いノートのタイトル".chars().cycle().take(80).collect();
    let name = format!("{title}.md");
    assert_eq!(name.len(), 243);
    let (_d, v) = setup(&[(&name, "external")]);
    let n = v.read_note(&name).unwrap();
    let r = v.write_note(&name, "edited", Some(&n.hash));
    assert!(r.is_ok(), "save failed: {:?}", r.err());
}

#[test]
fn unique_path_handles_names_at_the_limit_without_panicking() {
    let (_d, v) = setup(&[]);
    let base = "c".repeat(250);
    assert_eq!(v.unique_path("", &base, "md").unwrap().len(), 253);
}

// ---------------------------------------------------------------------------
// FINDING-051: temp files get a random name and are created with O_EXCL
// ---------------------------------------------------------------------------

#[cfg(unix)] // Symlinks need Developer Mode or admin rights on Windows.
#[test]
fn fs08_planted_temp_symlink_does_not_redirect_write() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("victim.txt"), "victim original").unwrap();
    let (d, v) = setup(&[("note.md", "n")]);
    // Temp names used to be `.<name>.cairn-tmp-<pid>`: hidden and guessable.
    // Plant a symlink at that name; the write must not follow it.
    let tmp = format!(".note.md.cairn-tmp-{}", std::process::id());
    symlink(outside.path().join("victim.txt"), d.path().join(&tmp)).unwrap();
    let n = v.read_note("note.md").unwrap();
    let r = v.write_note("note.md", "note text the attacker wants", Some(&n.hash));
    let victim = fs::read_to_string(outside.path().join("victim.txt")).unwrap();
    let is_link = fs::symlink_metadata(d.path().join("note.md")).unwrap().file_type().is_symlink();
    assert_eq!(victim, "victim original", "file outside the vault was overwritten (write result {r:?})");
    assert!(!is_link, "note.md was replaced by the attacker's symlink");
}

// ---------------------------------------------------------------------------
// FINDING-044: read-only notes are not overwritten
// ---------------------------------------------------------------------------

#[test]
fn fs09_read_only_note_is_not_overwritten() {
    let (d, v) = setup(&[("ro.md", "read only")]);
    // Read-only on every platform: no write bits on Unix, the read-only
    // flag on Windows.
    let ro = d.path().join("ro.md");
    let writable = fs::metadata(&ro).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    fs::set_permissions(&ro, read_only).unwrap();
    let n = v.read_note("ro.md").unwrap();
    let r = v.write_note("ro.md", "overwritten", Some(&n.hash));
    let now = read(&d, "ro.md");
    fs::set_permissions(&ro, writable).unwrap();
    assert!(r.is_err(), "write to a read-only file succeeded");
    assert_eq!(now, "read only");
}

#[cfg(unix)] // Unix mode bits. Windows has only a read-only flag.
#[test]
fn save_keeps_the_file_mode() {
    let (d, v) = setup(&[("private.md", "p"), ("exec.md", "e")]);
    chmod(&d.path().join("private.md"), 0o600);
    chmod(&d.path().join("exec.md"), 0o750);
    for (p, mode) in [("private.md", 0o600), ("exec.md", 0o750)] {
        let n = v.read_note(p).unwrap();
        v.write_note(p, "changed", Some(&n.hash)).unwrap();
        assert_eq!(fs::metadata(d.path().join(p)).unwrap().mode() & 0o777, mode, "{p}");
    }
}

#[cfg(unix)] // Unix mode bits. Windows has only a read-only flag.
#[test]
fn fs23_private_note_temp_file_is_not_world_readable() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (d, v) = setup(&[("private.md", "secret")]);
    chmod(&d.path().join("private.md"), 0o600);
    let stop = Arc::new(AtomicBool::new(false));
    let (s2, root) = (stop.clone(), d.path().to_path_buf());
    let watcher = std::thread::spawn(move || {
        let mut modes = std::collections::BTreeSet::new();
        while !s2.load(Ordering::Relaxed) {
            for e in fs::read_dir(&root).into_iter().flatten().flatten() {
                if e.file_name().to_string_lossy().contains("cairn-tmp")
                    && let Ok(m) = e.metadata()
                {
                    modes.insert(m.mode() & 0o777);
                }
            }
        }
        modes
    });
    let big = "secret line\n".repeat(2_000_000); // 24 MB so the temp file lives long enough to observe
    for _ in 0..3 {
        v.write_note("private.md", &big, None).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    let modes = watcher.join().unwrap();
    assert!(!modes.is_empty(), "never saw the temp file; rerun");
    assert!(modes.iter().all(|m| m & 0o077 == 0), "temp file modes seen: {:?}", modes.iter().map(|m| format!("{m:o}")).collect::<Vec<_>>());
}

#[cfg(unix)] // Windows ignores the read-only flag on a folder.
#[test]
fn read_only_folder_fails_cleanly_without_leftovers() {
    let (d, v) = setup(&[("rodir/n.md", "in ro dir"), ("ok.md", "ok")]);
    chmod(&d.path().join("rodir"), 0o555);
    let n = v.read_note("rodir/n.md").unwrap();
    let w = v.write_note("rodir/n.md", "x", Some(&n.hash));
    let c = v.create_note("rodir/new.md", "x");
    let r = v.rename("rodir/n.md", "moved.md");
    let del = v.delete("rodir/n.md");
    let left = names(&d.path().join("rodir"));
    chmod(&d.path().join("rodir"), 0o755);
    assert!(matches!(w, Err(CoreError::Io(_))), "{w:?}");
    assert!(matches!(c, Err(CoreError::Io(_))), "{c:?}");
    assert!(matches!(r, Err(CoreError::Io(_))), "{r:?}");
    assert!(matches!(del, Err(CoreError::Io(_))), "{del:?}");
    assert_eq!(left, vec!["n.md"], "temp files left behind");
    assert_eq!(read(&d, "rodir/n.md"), "in ro dir");
    assert!(v.index().entry("rodir/n.md").is_some());
    assert!(v.index().entry("rodir/new.md").is_none());
    assert!(v.index().entry("moved.md").is_none());
    assert!(v.rescan().unwrap().is_empty(), "index drifted from disk");
}

// ---------------------------------------------------------------------------
// FINDING-046: write_note's conflict check does not trust size+mtime
// ---------------------------------------------------------------------------

#[test]
fn fs10_same_size_same_mtime_external_edit_is_not_overwritten() {
    let (d, v) = setup(&[]);
    let r = v.create_note("n.md", "AAAA original").unwrap();
    let mtime = fs::metadata(d.path().join("n.md")).unwrap().modified().unwrap();
    // e.g. `rsync -t`, `cp -p`, `tar x`, a backup restore: same size, mtime preserved.
    fs::write(d.path().join("n.md"), "BBBB external").unwrap();
    fs::OpenOptions::new().write(true).open(d.path().join("n.md")).unwrap().set_modified(mtime).unwrap();
    // The editor still has the old base hash and saves.
    let w = v.write_note("n.md", "AAAA original + my edit", Some(&r.hash));
    assert!(matches!(w, Err(CoreError::Conflict(_))), "stale write accepted: {w:?}");
    assert_eq!(read(&d, "n.md"), "BBBB external");
}

#[test]
fn write_note_base_hash_rules_hold() {
    let (d, v) = setup(&[("n.md", "v1")]);
    let n = v.read_note("n.md").unwrap();
    fs::write(d.path().join("n.md"), "v2 external, longer").unwrap();
    assert!(matches!(v.write_note("n.md", "mine", Some(&n.hash)), Err(CoreError::Conflict(_))));
    assert_eq!(read(&d, "n.md"), "v2 external, longer");
    fs::remove_file(d.path().join("n.md")).unwrap();
    assert!(matches!(v.write_note("n.md", "mine", Some(&n.hash)), Err(CoreError::Conflict(_))));
    assert!(!d.path().join("n.md").exists(), "write recreated a deleted file");
    assert!(matches!(v.write_note("gone/x.md", "mine", Some(&n.hash)), Err(CoreError::Conflict(_))));
    assert!(!d.path().join("gone").exists());
    // bad hash strings are rejected, not panicking
    for bad in ["", "zz", &"g".repeat(64), &"0".repeat(63)] {
        assert!(v.write_note("x.md", "y", Some(bad)).is_err(), "{bad:?}");
    }
}

// ---------------------------------------------------------------------------
// FINDING-139: a multi-byte base hash is rejected without a panic
// ---------------------------------------------------------------------------

#[test]
fn fs19_multibyte_base_hash_does_not_panic() {
    let (_d, v) = setup(&[("note.md", "n")]);
    let bad = format!("{}a", "\u{20ac}".repeat(21));
    assert_eq!(bad.len(), 64);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.write_note("note.md", "y", Some(&bad)).map(|_| ())));
    assert!(r.is_ok(), "write_note panicked");
    assert!(r.unwrap().is_err());
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.write_file("note.md", b"y", Some(&bad)).map(|_| ())));
    assert!(r.is_ok(), "write_file panicked");
}

// ---------------------------------------------------------------------------
// Trash
// ---------------------------------------------------------------------------

#[test]
fn vault_trash_never_overwrites_earlier_deletes() {
    let (d, v) = setup(&[("a/x.md", "a x"), ("b/x.md", "b x"), ("x.md", "root x"), ("x 1.md", "already x 1"), ("x 1/x.md", "folder named x 1")]);
    for p in ["a/x.md", "b/x.md", "x.md", "x 1.md", "x 1"] {
        v.delete(p).unwrap();
    }
    v.create_note("x.md", "second root x").unwrap();
    v.delete("x.md").unwrap();
    v.create_note("x.md", "third").unwrap();
    v.delete("x.md").unwrap();
    let t = d.path().join(".trash");
    let mut contents: Vec<String> = Vec::new();
    for n in names(&t) {
        let p = t.join(&n);
        if p.is_dir() {
            contents.push(fs::read_to_string(p.join("x.md")).unwrap());
        } else {
            contents.push(fs::read_to_string(p).unwrap());
        }
    }
    contents.sort();
    assert_eq!(contents, vec!["a x", "already x 1", "b x", "folder named x 1", "root x", "second root x", "third"]);
}

#[test]
fn delete_folder_moves_hidden_files_with_it() {
    let (d, v) = setup(&[("dir/.hidden", "hid"), ("dir/.git/HEAD", "ref"), ("dir/n.md", "n")]);
    let c = v.delete("dir").unwrap();
    assert_eq!(c, vec![Change::Deleted { path: "dir".into(), kind: cairn_core::EntryKind::Dir }]);
    assert!(!d.path().join("dir").exists());
    assert_eq!(names(&d.path().join(".trash/dir")), vec![".git", ".hidden", "n.md"]);
    assert!(v.entries().is_empty());
}

#[test]
fn delete_twice_and_missing_paths() {
    let (_d, v) = setup(&[("a.md", "a")]);
    v.delete("a.md").unwrap();
    assert!(matches!(v.delete("a.md"), Err(CoreError::NotFound(_))));
    assert!(matches!(v.delete(""), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.delete("/"), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.delete(".."), Err(CoreError::InvalidPath(_))));
}

// ---------------------------------------------------------------------------
// FINDING-022 / FINDING-135: hidden paths through read_note / write_note
// ---------------------------------------------------------------------------

#[test]
fn fs13_read_note_refuses_hidden_files() {
    let (_d, v) = setup(&[(".git/config", "[remote \"origin\"]\nurl = https://user:ghp_secret@example.com/x"), ("n.md", "n")]);
    let r = v.read_note(".git/config");
    assert!(matches!(r, Err(CoreError::InvalidPath(_)) | Err(CoreError::NotANote(_))), "read_note returned {:?}", r.map(|n| n.content));
    let r = v.read_file(".git/config");
    assert!(matches!(r, Err(CoreError::InvalidPath(_))), "read_file returned {:?}", r.map(|b| String::from_utf8_lossy(&b).into_owned()));
}

#[test]
fn fs14_write_note_into_hidden_folder_is_refused() {
    let (d, v) = setup(&[(".trash/old.md", "trashed")]);
    let w = v.write_note(".trash/old.md", "overwritten", None);
    let in_index = v.index().entry(".trash/old.md").is_some();
    let hits = v.search("overwritten", 10).len();
    let phantom = v.rescan().unwrap();
    assert!(matches!(w, Err(CoreError::InvalidPath(_))), "write_note returned {:?}", w.map(|r| r.changes));
    assert_eq!(read(&d, ".trash/old.md"), "trashed");
    assert!(!in_index && hits == 0 && phantom.is_empty(), "index polluted: in_index={in_index} hits={hits} rescan={phantom:?}");
}

#[test]
fn fs21_names_cairn_refuses_cannot_be_created() {
    let (d, v) = setup(&[]);
    // create_note validates the folders it adds, not only the last component.
    let a = v.create_note("bad:dir/x.md", "");
    let b = v.create_note(" lead/x.md", "");
    let c = v.create_note("trail./x.md", "");
    let g = v.create_folder("ok/bad:dir/sub");
    let h = v.create_file("[x]/pic.png", b"png");
    let made: Vec<String> = names(d.path());
    assert!(a.is_err() && b.is_err() && c.is_err() && g.is_err() && h.is_err(), "created: {made:?}");
}

#[test]
fn fs21_write_note_cannot_create_names_cairn_refuses() {
    let (d, v) = setup(&[]);
    // write_note checks a new file's name with validate_name too.
    let e = v.write_note("bad:name.md", "", None);
    let f = v.write_note("[x].md", "", None);
    let made: Vec<String> = names(d.path());
    assert!(e.is_err() && f.is_err(), "created: {made:?}");
}

#[test]
fn fs21_write_note_checks_new_folders_and_keeps_existing_names() {
    // Like create_note: a folder a new note adds needs a valid name too,
    // while files and folders already on disk keep theirs.
    let (d, v) = setup(&[("[old].md", "old"), ("Odd [dir]/a.md", "a")]);
    let r = v.write_note("ok/bad:dir/x.md", "", None);
    // On Windows the file system refuses the colon first.
    let refused = if cfg!(windows) {
        matches!(&r, Err(CoreError::Io(m)) if m.starts_with("Windows does not allow"))
    } else {
        matches!(r, Err(CoreError::InvalidName(_)))
    };
    assert!(refused, "write_note returned {:?}", r.map(|r| r.changes));
    assert_eq!(names(d.path()), ["Odd [dir]", "[old].md"]);
    let w = v.write_note("[old].md", "forced", None).unwrap();
    let w = v.write_note("[old].md", "saved", Some(&w.hash)).unwrap();
    v.write_note("Odd [dir]/new.md", "new", None).unwrap();
    assert_eq!((read(&d, "[old].md"), read(&d, "Odd [dir]/new.md")), ("saved".into(), "new".into()));
    // Deleted by another program: a save is still a conflict, and the
    // note comes back under its old name through recreate_note.
    fs::remove_file(d.path().join("[old].md")).unwrap();
    let r = v.write_note("[old].md", "edited", Some(&w.hash));
    assert!(matches!(r, Err(CoreError::Conflict(_))), "write_note returned {:?}", r.map(|r| r.changes));
    v.recreate_note("[old].md", "edited").unwrap();
    assert_eq!(read(&d, "[old].md"), "edited");
}

#[test]
fn fs21_new_entries_in_folders_with_names_cairn_refuses_are_allowed() {
    // Folders made outside Cairn keep their names; only new ones are checked.
    let (d, v) = setup(&[("Odd [dir]/a.md", "a")]);
    v.create_note("Odd [dir]/x.md", "").unwrap();
    v.create_note("Odd [dir]/new/y.md", "").unwrap();
    v.create_folder("Odd [dir]/sub").unwrap();
    // Through the vault's canonical root: on Windows that is a \\?\ path,
    // without which Windows drops the trailing dot.
    let root = StdFs::new(d.path(), TrashMode::Vault).unwrap().root().to_path_buf();
    fs::create_dir(root.join("trail.")).unwrap();
    v.create_note("trail./z.md", "").unwrap();
    assert!(d.path().join("Odd [dir]/new/y.md").is_file() && root.join("trail./z.md").is_file());
}

// ---------------------------------------------------------------------------
// FINDING-053: case-insensitive duplicates are detected on create
// ---------------------------------------------------------------------------

#[test]
fn fs22_create_refuses_case_insensitive_duplicate() {
    let (_d, v) = setup(&[("note.md", "n"), ("Folder/a.md", "a")]);
    let r = v.create_note("Note.md", "x");
    assert!(matches!(r, Err(CoreError::AlreadyExists(_))), "create_note returned {:?}", r.map(|r| r.entry.path));
    let r = v.create_folder("folder");
    assert!(matches!(r, Err(CoreError::AlreadyExists(_))), "create_folder returned {r:?}");
}

// ---------------------------------------------------------------------------
// FINDING-142: concurrent write_config to the same file
// ---------------------------------------------------------------------------

#[test]
fn fs24_concurrent_write_config_succeeds() {
    let (_d, v) = setup(&[]);
    let v = Arc::new(v);
    let hs: Vec<_> = (0..4u8)
        .map(|t| {
            let v = v.clone();
            std::thread::spawn(move || {
                let mut errs = Vec::new();
                for i in 0..100 {
                    let c = format!("{{\"t\":{t},\"i\":{i},\"pad\":\"{}\"}}", "x".repeat(50_000 + i * 100));
                    if let Err(e) = v.write_config("settings.json", &c) {
                        errs.push(e.to_string());
                    }
                }
                errs
            })
        })
        .collect();
    let errs: Vec<String> = hs.into_iter().flat_map(|h| h.join().unwrap()).collect();
    assert!(errs.is_empty(), "{} of 400 writes failed, e.g. {:?}", errs.len(), errs.first());
}

// ---------------------------------------------------------------------------
// Path validation through every public entry point
// ---------------------------------------------------------------------------

#[test]
fn path_validation_through_every_entry_point() {
    let (d, v) = setup(&[("keep.md", "keep")]);
    let escapes = ["..", "../x.md", "a/../../x.md", "..\\x.md", ".", "./a.md", "a/./b.md"];
    for p in escapes {
        assert!(matches!(v.create_note(p, ""), Err(CoreError::InvalidPath(_))), "create_note {p}");
        assert!(matches!(v.write_note(p, "", None), Err(CoreError::InvalidPath(_)) | Err(CoreError::NotANote(_))), "write_note {p}");
        assert!(matches!(v.write_file(p, b"", None), Err(CoreError::InvalidPath(_))), "write_file {p}");
        assert!(matches!(v.create_file(p, b""), Err(CoreError::InvalidPath(_))), "create_file {p}");
        assert!(matches!(v.create_folder(p), Err(CoreError::InvalidPath(_))), "create_folder {p}");
        assert!(matches!(v.ensure_folder(p), Err(CoreError::InvalidPath(_))), "ensure_folder {p}");
        assert!(matches!(v.prune_empty_folders(p), Err(CoreError::InvalidPath(_))), "prune {p}");
        assert!(matches!(v.read_note(p), Err(CoreError::InvalidPath(_))), "read_note {p}");
        assert!(matches!(v.read_file(p), Err(CoreError::InvalidPath(_))), "read_file {p}");
        assert!(matches!(v.delete(p), Err(CoreError::InvalidPath(_))), "delete {p}");
        assert!(matches!(v.rename(p, "z.md"), Err(CoreError::InvalidPath(_))), "rename from {p}");
        assert!(matches!(v.rename("keep.md", p), Err(CoreError::InvalidPath(_))), "rename to {p}");
        assert!(matches!(v.unique_path(p, "x", "md"), Err(CoreError::InvalidPath(_))), "unique_path {p}");
    }
    // Root / empty
    for p in ["", "/", "//"] {
        assert!(v.create_note(p, "").is_err());
        assert!(v.create_folder(p).is_err());
        assert!(v.delete(p).is_err());
        assert!(v.write_file(p, b"", None).is_err());
    }
    // NUL bytes give clean errors
    for p in ["a\0b.md", "\0"] {
        assert!(v.create_note(p, "").is_err());
        assert!(v.write_note(p, "", None).is_err());
        assert!(v.read_note(p).is_err());
        assert!(v.delete(p).is_err());
    }
    // Hidden segments are refused for creation
    for p in [".cairn/x.md", "a/.git/x.md", ".trash/x.md"] {
        assert!(matches!(v.create_note(p, ""), Err(CoreError::InvalidPath(_))), "{p}");
        assert!(matches!(v.create_folder(p), Err(CoreError::InvalidPath(_))), "{p}");
        assert!(matches!(v.create_file(p, b""), Err(CoreError::InvalidPath(_))), "{p}");
        assert!(matches!(v.write_file(p, b"", None), Err(CoreError::InvalidPath(_))), "{p}");
    }
    // Absolute OS paths are re-rooted inside the vault.
    let r = v.create_note("/tmp/escape-test-cairn.md", "x").unwrap();
    assert_eq!(r.entry.path, "tmp/escape-test-cairn.md");
    assert!(d.path().join("tmp/escape-test-cairn.md").exists());
    assert!(!Path::new("/tmp/escape-test-cairn.md").exists());
    assert_eq!(read(&d, "keep.md"), "keep");
}

#[test]
fn config_names_cannot_escape_the_cairn_folder() {
    let (d, v) = setup(&[]);
    for n in ["../x", "a/../../x", "..\\x", ".", "", "a/.b", ".hidden", "snippets/../../.git/config", "..", "/.."] {
        assert!(matches!(v.read_config(n), Err(CoreError::InvalidPath(_))), "read {n:?}");
        assert!(matches!(v.write_config(n, "c"), Err(CoreError::InvalidPath(_))), "write {n:?}");
        assert!(matches!(v.list_config(n), Err(CoreError::InvalidPath(_))), "list {n:?}");
    }
    v.write_config("/etc/passwd-cairn-test", "c").unwrap();
    assert!(d.path().join(".cairn/etc/passwd-cairn-test").exists());
    assert!(!Path::new("/etc/passwd-cairn-test").exists());
    assert!(v.write_config("x\0y", "c").is_err());
    assert_eq!(names(d.path()), vec![".cairn"]);
}

// ---------------------------------------------------------------------------
// Content round trips
// ---------------------------------------------------------------------------

#[test]
fn non_utf8_notes_are_reported_and_never_rewritten() {
    let latin1: &[u8] = b"caf\xe9 au lait";
    let utf16: &[u8] = b"\xff\xfeh\x00i\x00";
    let binary: &[u8] = &[0u8, 159, 146, 150, 0, 255];
    let d = tempfile::tempdir().unwrap();
    for (n, b) in [("latin1.md", latin1), ("utf16.md", utf16), ("bin.md", binary)] {
        fs::write(d.path().join(n), b).unwrap();
    }
    let v = open(d.path());
    assert_eq!(v.entries().len(), 3, "vault opened with all three files listed");
    for (n, b) in [("latin1.md", latin1), ("utf16.md", utf16), ("bin.md", binary)] {
        assert!(matches!(v.read_note(n), Err(CoreError::Io(_))), "{n}");
        assert_eq!(v.read_file(n).unwrap(), b);
    }
    v.rescan().unwrap();
    v.rename("latin1.md", "moved.md").unwrap();
    for (n, b) in [("moved.md", latin1), ("utf16.md", utf16), ("bin.md", binary)] {
        assert_eq!(fs::read(d.path().join(n)).unwrap(), b, "{n} bytes changed");
    }
}

#[test]
fn crlf_bom_cr_and_empty_round_trip_byte_exact() {
    let files = [("crlf.md", "line1\r\nline2\r\n"), ("bom.md", "\u{feff}# Title\nbody"), ("cr.md", "old mac\rline"), ("empty.md", "")];
    let (d, v) = setup(&files);
    for (p, c) in files {
        let n = v.read_note(p).unwrap();
        assert_eq!(n.content, c);
        v.write_note(p, &n.content, Some(&n.hash)).unwrap();
        assert_eq!(fs::read(d.path().join(p)).unwrap(), c.as_bytes(), "{p}");
    }
    assert!(v.rescan().unwrap().is_empty());
}

#[test]
fn odd_names_created_outside_are_readable_and_writable() {
    let mut names_ = vec!["a#b.md", "x[1].md", "c^d.md", " lead.md", "trail .md", "dots..md", "zw\u{200b}j.md", "emoji 🪨.md"];
    // Windows refuses these characters in a name, and a colon there names a
    // stream of the file.
    if cfg!(not(windows)) {
        names_.extend(["p|q.md", "co:lon.md", "st*r.md", "q?.md", "quo\"te.md", "lt<gt>.md", "tab\there.md", "nl\nname.md"]);
    }
    let d = tempfile::tempdir().unwrap();
    for &n in &names_ {
        fs::write(d.path().join(n), format!("content of {n}")).unwrap();
    }
    let v = open(d.path());
    for &n in &names_ {
        let c = v.read_note(n).unwrap();
        v.write_note(n, &format!("{} + edit", c.content), Some(&c.hash)).unwrap();
        assert_eq!(fs::read_to_string(d.path().join(n)).unwrap(), format!("content of {n} + edit"));
        // Cairn refuses these names for new entries, but can rename away from them.
        let to = format!("renamed-{}", n.replace(['#', '[', ']', '^', '|', ':', '*', '?', '"', '<', '>', '\t', '\n'], "_"));
        v.rename(n, &to).unwrap();
        assert!(v.read_note(&to).is_ok());
    }
    assert!(v.rescan().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// prune_empty_folders / ensure_folder
// ---------------------------------------------------------------------------

#[test]
fn prune_keeps_folders_with_hidden_files_and_stops_at_non_empty() {
    let (d, v) = setup(&[("a/b/c/.keep", ""), ("x/y/z/n.md", "n"), ("x/sibling.md", "s"), ("h/.git/HEAD", "r")]);
    fs::create_dir_all(d.path().join("e1/e2/e3")).unwrap();
    v.rescan().unwrap();
    assert!(v.prune_empty_folders("a/b/c").unwrap().is_empty());
    assert!(d.path().join("a/b/c/.keep").exists());
    assert!(v.prune_empty_folders("h").unwrap().is_empty());
    fs::remove_file(d.path().join("x/y/z/n.md")).unwrap();
    v.rescan().unwrap();
    let c = v.prune_empty_folders("x/y/z").unwrap();
    assert_eq!(c.len(), 2, "{c:?}"); // x/y/z and x/y, not x (has sibling.md)
    assert!(d.path().join("x/sibling.md").exists());
    // pruning starts at the given folder only: unrelated empty folders stay
    assert!(d.path().join("e1/e2/e3").exists());
    assert!(v.prune_empty_folders(".trash").unwrap().is_empty());
    assert!(v.ensure_folder("x").unwrap().is_empty());
    assert!(v.ensure_folder("").unwrap().is_empty());
    assert_eq!(v.ensure_folder("new/deep").unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// FINDING-047: an external save while the temp file is written is not lost
// ---------------------------------------------------------------------------

#[test]
fn fs26_external_save_during_temp_write_is_not_lost() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (d, v) = setup(&[("n.md", "original")]);
    let n = v.read_note("n.md").unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let (done2, root) = (done.clone(), d.path().to_path_buf());
    // Another editor saves n.md right after Cairn passed its conflict check
    // (the moment Cairn's temp file appears).
    let other_editor = std::thread::spawn(move || {
        while !done2.load(Ordering::Relaxed) {
            let tmp_seen = fs::read_dir(&root).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("cairn-tmp"));
            if tmp_seen {
                fs::write(root.join("n.md"), "the other editor's save").unwrap();
                return true;
            }
        }
        false
    });
    let big = "my edit\n".repeat(3_000_000); // 24 MB so the window is wide enough to hit
    let r = v.write_note("n.md", &big, Some(&n.hash));
    done.store(true, Ordering::Relaxed);
    let raced = other_editor.join().unwrap();
    assert!(raced, "the other editor never saw the temp file; rerun");
    let disk = fs::read_to_string(d.path().join("n.md")).unwrap();
    let lost = disk != "the other editor's save" && r.is_ok();
    assert!(!lost, "write_note returned Ok and the other editor's save is gone (disk starts {:?})", &disk[..20.min(disk.len())]);
}
