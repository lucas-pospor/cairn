use std::fs;
use std::sync::Arc;

use cairn_core::{Change, CoreError, EntryKind, StdFs, TrashMode, Vault};

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

fn paths(v: &Vault) -> Vec<String> {
    let mut p: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    p.sort();
    p
}

#[test]
fn open_indexes_everything_but_hidden() {
    let (_d, v) = setup(&[
        ("a.md", "[[b]]"),
        ("dir/b.md", "# B"),
        ("dir/pic.png", "png"),
        (".cairn/settings.json", "{}"),
        (".git/HEAD", "x"),
    ]);
    assert_eq!(paths(&v), vec!["a.md", "dir", "dir/b.md", "dir/pic.png"]);
    assert_eq!(v.backlinks("dir/b.md")[0].source, "a.md");
}

#[test]
fn create_write_read_note() {
    let (d, v) = setup(&[]);
    let r = v.create_note("new/deep/Note.md", "hello [[x]]").unwrap();
    // parents created and reported first
    let created: Vec<_> = r
        .changes
        .iter()
        .map(|c| match c {
            Change::Created { entry } => entry.path.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(created, vec!["new", "new/deep", "new/deep/Note.md"]);
    assert_eq!(fs::read_to_string(d.path().join("new/deep/Note.md")).unwrap(), "hello [[x]]");
    let n = v.read_note("new/deep/Note.md").unwrap();
    assert_eq!(n.hash, r.hash);
    assert!(matches!(v.create_note("new/deep/Note.md", ""), Err(CoreError::AlreadyExists(_))));
    assert!(matches!(v.create_note("bad:name.md", ""), Err(CoreError::InvalidName(_))));
    assert!(matches!(v.create_note(".hidden/x.md", ""), Err(CoreError::InvalidPath(_))));
    assert!(matches!(v.create_note("x.txt", ""), Err(CoreError::NotANote(_))));
}

#[test]
fn write_detects_external_change() {
    let (d, v) = setup(&[("n.md", "v1")]);
    let n = v.read_note("n.md").unwrap();
    let w = v.write_note("n.md", "v2", Some(&n.hash)).unwrap();
    // someone else edits the file
    fs::write(d.path().join("n.md"), "external").unwrap();
    let err = v.write_note("n.md", "v3", Some(&w.hash)).unwrap_err();
    assert_eq!(err, CoreError::Conflict("n.md".into()));
    assert_eq!(fs::read_to_string(d.path().join("n.md")).unwrap(), "external");
    // forced write without base hash
    v.write_note("n.md", "v3", None).unwrap();
    assert_eq!(fs::read_to_string(d.path().join("n.md")).unwrap(), "v3");
    // deleted underneath us
    let h = v.read_note("n.md").unwrap().hash;
    fs::remove_file(d.path().join("n.md")).unwrap();
    assert!(matches!(v.write_note("n.md", "v4", Some(&h)), Err(CoreError::Conflict(_))));
}

#[test]
fn recreate_note_never_replaces_a_file_that_came_back() {
    let (d, v) = setup(&[("n.md", "v1"), ("f/m.md", "m")]);
    // gone: written again, with its folder
    fs::remove_file(d.path().join("n.md")).unwrap();
    fs::remove_dir_all(d.path().join("f")).unwrap();
    v.rescan().unwrap();
    let r = v.recreate_note("n.md", "mine").unwrap();
    assert_eq!(fs::read_to_string(d.path().join("n.md")).unwrap(), "mine");
    assert_eq!(v.read_note("n.md").unwrap().hash, r.hash);
    v.recreate_note("f/m.md", "mine m").unwrap();
    assert_eq!(fs::read_to_string(d.path().join("f/m.md")).unwrap(), "mine m");
    // back on disk (not indexed yet): left alone
    fs::remove_file(d.path().join("n.md")).unwrap();
    v.rescan().unwrap();
    fs::write(d.path().join("n.md"), "came back").unwrap();
    assert_eq!(v.recreate_note("n.md", "mine 2").unwrap_err(), CoreError::Conflict("n.md".into()));
    assert_eq!(fs::read_to_string(d.path().join("n.md")).unwrap(), "came back");
    // a folder in its place: left alone too
    fs::create_dir(d.path().join("g.md")).unwrap();
    assert!(matches!(v.recreate_note("g.md", "x"), Err(CoreError::Conflict(_))));
    assert!(matches!(v.recreate_note("x.txt", ""), Err(CoreError::NotANote(_))));
}

#[test]
fn rename_and_move() {
    let (d, v) = setup(&[("a.md", "x"), ("f/b.md", "[[a]]"), ("g/.keep", "")]);
    v.rename("a.md", "A2.md").unwrap();
    assert!(d.path().join("A2.md").exists());
    // [[a]] no longer resolves
    assert!(v.outgoing("f/b.md")[0].resolved.is_none());
    // move a folder into another folder
    let ch = v.rename("f", "g/f").unwrap();
    assert_eq!(ch.len(), 1);
    assert!(matches!(&ch[0], Change::Renamed { from, entry } if from == "f" && entry.path == "g/f"));
    assert_eq!(paths(&v), vec!["A2.md", "g", "g/f", "g/f/b.md"]);
    assert!(matches!(v.rename("g", "g/f/g"), Err(CoreError::MoveIntoSelf(_))));
    assert!(matches!(v.rename("A2.md", "nope/A2.md"), Err(CoreError::NotFound(_))));
    v.create_note("c.md", "").unwrap();
    assert!(matches!(v.rename("A2.md", "c.md"), Err(CoreError::AlreadyExists(_))));
}

#[test]
fn delete_goes_to_trash() {
    let (d, v) = setup(&[("f/a.md", "x"), ("f/b.md", "y"), ("c.md", "[[a]]")]);
    let ch = v.delete("f").unwrap();
    assert_eq!(ch, vec![Change::Deleted { path: "f".into(), kind: EntryKind::Dir }]);
    assert_eq!(paths(&v), vec!["c.md"]);
    assert!(d.path().join(".trash/f/a.md").exists());
    assert!(v.backlinks("f/a.md").is_empty());
}

#[test]
fn unique_paths() {
    let (_d, v) = setup(&[("Untitled.md", ""), ("Untitled 1.md", "")]);
    assert_eq!(v.unique_path("", "Untitled", "md").unwrap(), "Untitled 2.md");
    assert_eq!(v.unique_path("sub", "Untitled", "md").unwrap(), "sub/Untitled.md");
}

#[test]
fn create_refuses_names_that_differ_only_in_case() {
    let (d, v) = setup(&[("untitled.md", ""), ("Dir/Über.png", "png")]);
    // "New note" skips the twin instead of failing on it.
    assert_eq!(v.unique_path("", "Untitled", "md").unwrap(), "Untitled 1.md");
    let r = v.create_file("dir/x.png", b"x");
    assert_eq!(r.map(|w| w.entry.path), Err(CoreError::AlreadyExists("Dir".into())));
    let r = v.create_file("Dir/über.png", b"x");
    assert_eq!(r.map(|w| w.entry.path), Err(CoreError::AlreadyExists("Dir/Über.png".into())));
    // A different name, and the same name in another folder, are fine.
    v.create_file("Dir/sub/über.png", b"x").unwrap();
    v.create_note("Dir/untitled.md", "").unwrap();
    assert!(!d.path().join("dir").exists());
}

#[test]
fn renames_in_the_app_refuse_names_that_differ_only_in_case() {
    let (d, v) = setup(&[
        ("Note.md", "n"),
        ("Untitled.md", ""),
        ("Dir/a.md", "a"),
        ("Projects/x.md", "x"),
        ("Other/y.md", "y"),
        ("Twins/NOTE.md", ""),
        ("Twins/note.md", ""),
    ]);
    // F2, a move (drag and drop, Move to) and a folder.
    assert_eq!(v.check_rename("Untitled.md", "note.md"), Err(CoreError::AlreadyExists("Note.md".into())));
    assert_eq!(v.check_rename("Untitled.md", "Dir/A.md"), Err(CoreError::AlreadyExists("Dir/a.md".into())));
    assert_eq!(v.check_rename("Other", "projects"), Err(CoreError::AlreadyExists("Projects".into())));
    // The entry itself does not count, so a case-only rename still works,
    // unless another entry has the name too.
    v.check_rename("Note.md", "note.md").unwrap();
    v.check_rename("Projects", "projects").unwrap();
    let r = v.check_rename("Twins/NOTE.md", "Twins/Note.md");
    assert_eq!(r, Err(CoreError::AlreadyExists("Twins/note.md".into())));
    v.check_rename("Untitled.md", "Dir/b.md").unwrap();
    // Vault::rename does not check: sync applies renames from other devices
    // with it.
    v.rename("Untitled.md", "note.md").unwrap();
    assert!(d.path().join("Note.md").exists() && d.path().join("note.md").exists());
}

#[test]
fn rescan_detects_external_changes() {
    let (d, v) = setup(&[("a.md", "alpha"), ("b.md", "beta"), ("dir/c.md", "gamma"), ("img.png", "IMG")]);
    let root = d.path();
    fs::write(root.join("new.md"), "fresh [[a]]").unwrap();
    fs::write(root.join("a.md"), "alpha changed").unwrap();
    fs::remove_file(root.join("b.md")).unwrap();
    fs::rename(root.join("dir"), root.join("dir2")).unwrap();
    fs::rename(root.join("img.png"), root.join("moved.png")).unwrap();

    let mut ch = v.rescan().unwrap();
    ch.sort_by_key(|c| format!("{c:?}"));
    let summary: Vec<String> = ch
        .iter()
        .map(|c| match c {
            Change::Created { entry } => format!("+{}", entry.path),
            Change::Modified { entry } => format!("~{}", entry.path),
            Change::Deleted { path, .. } => format!("-{path}"),
            Change::Renamed { from, entry } => format!("{from}->{}", entry.path),
        })
        .collect();
    let mut summary = summary;
    summary.sort();
    assert_eq!(summary, vec!["+new.md", "-b.md", "dir->dir2", "img.png->moved.png", "~a.md"]);
    assert_eq!(paths(&v), vec!["a.md", "dir2", "dir2/c.md", "moved.png", "new.md"]);
    assert_eq!(v.search("changed", 5)[0].path, "a.md");
    assert_eq!(v.backlinks("a.md")[0].source, "new.md");
    // a second scan finds nothing
    assert!(v.rescan().unwrap().is_empty());
}

#[test]
fn rescan_detects_note_rename_by_content() {
    let (d, v) = setup(&[("old name.md", "some content")]);
    fs::rename(d.path().join("old name.md"), d.path().join("new name.md")).unwrap();
    let ch = v.rescan_paths(&["old name.md".into(), "new name.md".into()]).unwrap();
    assert!(matches!(&ch[..], [Change::Renamed { from, entry }] if from == "old name.md" && entry.path == "new name.md"));
}

#[test]
fn rescan_pairs_identical_notes_by_name_then_folder() {
    let (d, v) = setup(&[("a.md", "template"), ("b.md", "template"), ("x/c.md", "template"), ("y/d.md", "template"), ("e.md", "")]);
    let root = d.path();
    fs::create_dir(root.join("archive")).unwrap();
    for n in ["a.md", "b.md", "e.md"] {
        fs::rename(root.join(n), root.join("archive").join(n)).unwrap();
    }
    fs::rename(root.join("x/c.md"), root.join("x/c2.md")).unwrap();
    fs::rename(root.join("y/d.md"), root.join("y/d2.md")).unwrap();
    let mut got: Vec<String> = v
        .rescan()
        .unwrap()
        .into_iter()
        .filter_map(|c| match c {
            Change::Renamed { from, entry } => Some(format!("{from}->{}", entry.path)),
            _ => None,
        })
        .collect();
    got.sort();
    // Moved notes keep their names, renamed ones their folders. An empty
    // note has no content to follow: deleted and created.
    assert_eq!(got, ["a.md->archive/a.md", "b.md->archive/b.md", "x/c.md->x/c2.md", "y/d.md->y/d2.md"]);
    assert!(v.rescan().unwrap().is_empty());
}

#[test]
fn stale_hint_inside_a_renamed_subfolder_does_not_split_the_rename() {
    // The watcher may hand over an earlier event for a file in a folder (a
    // read, a save) in its own batch, after the folder was renamed.
    let (d, v) = setup(&[("a/dir/C.md", "c"), ("a/Z.md", "z")]);
    fs::rename(d.path().join("a/dir"), d.path().join("a/dir2")).unwrap();
    let first = v.rescan_paths(&["a/dir/C.md".into()]).unwrap();
    assert!(matches!(&first[..], [Change::Renamed { from, entry }] if from == "a/dir" && entry.path == "a/dir2"), "{first:?}");
    assert!(v.rescan_paths(&["a/dir".into(), "a/dir2".into()]).unwrap().is_empty());
    assert_eq!(paths(&v), vec!["a", "a/Z.md", "a/dir2", "a/dir2/C.md"]);
    // A file deleted from a folder that is still there is a plain deletion.
    fs::remove_file(d.path().join("a/Z.md")).unwrap();
    let ch = v.rescan_paths(&["a/Z.md".into()]).unwrap();
    assert!(matches!(&ch[..], [Change::Deleted { path, .. }] if path == "a/Z.md"), "{ch:?}");
}

#[test]
fn own_writes_are_silent_on_rescan() {
    let (_d, v) = setup(&[("a.md", "1")]);
    let h = v.read_note("a.md").unwrap().hash;
    v.write_note("a.md", "2", Some(&h)).unwrap();
    assert!(v.rescan_paths(&["a.md".into()]).unwrap().is_empty());
    v.create_note("b.md", "x").unwrap();
    v.create_folder("f").unwrap();
    v.rename("b.md", "f/b.md").unwrap();
    assert!(v.rescan().unwrap().is_empty());
}

#[test]
fn rescan_paths_picks_up_new_folder_tree() {
    let (d, v) = setup(&[]);
    fs::create_dir_all(d.path().join("x/y")).unwrap();
    fs::write(d.path().join("x/y/n.md"), "n").unwrap();
    // watcher only told us about the file
    let ch = v.rescan_paths(&["x/y/n.md".into()]).unwrap();
    assert_eq!(ch.len(), 3);
    assert_eq!(paths(&v), vec!["x", "x/y", "x/y/n.md"]);
    // hidden paths are ignored
    fs::create_dir_all(d.path().join(".obsidian")).unwrap();
    assert!(v.rescan_paths(&[".obsidian".into()]).unwrap().is_empty());
}

#[test]
fn touch_without_content_change_is_silent() {
    let (d, v) = setup(&[("a.md", "same")]);
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(d.path().join("a.md"), "same").unwrap();
    assert!(v.rescan().unwrap().is_empty());
}

#[test]
fn config_files_live_in_hidden_folder() {
    let (d, v) = setup(&[("n.md", "")]);
    assert_eq!(v.read_config("settings.json").unwrap(), None);
    v.write_config("settings.json", "{\"a\":1}").unwrap();
    v.write_config("snippets/wide.css", "body{}").unwrap();
    assert_eq!(v.read_config("settings.json").unwrap().as_deref(), Some("{\"a\":1}"));
    assert_eq!(v.list_config("snippets").unwrap(), vec!["wide.css"]);
    assert!(d.path().join(".cairn/snippets/wide.css").exists());
    assert!(v.read_config("../n.md").is_err());
    assert!(v.read_config(".hidden").is_err());
    // never shows up in the index
    assert!(v.rescan().unwrap().is_empty());
    assert_eq!(paths(&v), vec!["n.md"]);
}

/// A received vault (git clone, archive) can link a config file to any file
/// of the user's. Config files follow a symlink only when the file it leads
/// to is in the vault; changing a setting replaces a link out of the vault
/// with a plain file and leaves the file outside alone.
#[cfg(unix)]
#[test]
fn config_files_follow_symlinks_only_inside_the_vault() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let outside = tempfile::tempdir().unwrap();
    let o = outside.path();
    fs::write(o.join("bashrc"), "export A=1\n").unwrap();
    fs::write(o.join("ro.json"), "ro").unwrap();
    fs::set_permissions(o.join("ro.json"), fs::Permissions::from_mode(0o444)).unwrap();
    fs::write(o.join("shared.json"), "shared").unwrap();
    fs::hard_link(o.join("shared.json"), o.join("shared-too.json")).unwrap();
    let (d, v) = setup(&[(".cairn/x", ""), ("config/hotkeys.json", "{}")]);
    let r = d.path();
    let is_link = |p: &str| fs::symlink_metadata(r.join(p)).unwrap().file_type().is_symlink();
    symlink(o.join("bashrc"), r.join(".cairn/settings.json")).unwrap();
    symlink(o.join("ro.json"), r.join(".cairn/ro.json")).unwrap();
    symlink(o.join("shared.json"), r.join(".cairn/workspace.json")).unwrap();
    // Through a link in the vault that leads out.
    symlink(o.join("bashrc"), r.join("config/out.json")).unwrap();
    symlink("../config/out.json", r.join(".cairn/chain.json")).unwrap();
    // A link that stays in the vault is followed.
    symlink("../config/hotkeys.json", r.join(".cairn/hotkeys.json")).unwrap();
    for name in ["settings.json", "ro.json", "workspace.json", "chain.json", "hotkeys.json"] {
        v.write_config(name, "{\"new\":1}").unwrap();
        assert_eq!(v.read_config(name).unwrap().as_deref(), Some("{\"new\":1}"), "{name}");
    }
    let read = |p: &std::path::Path| fs::read_to_string(p).unwrap();
    assert_eq!((read(&o.join("bashrc")), read(&o.join("ro.json"))), ("export A=1\n".into(), "ro".into()));
    assert_eq!((read(&o.join("shared.json")), read(&o.join("shared-too.json"))), ("shared".into(), "shared".into()));
    for p in [".cairn/settings.json", ".cairn/ro.json", ".cairn/workspace.json", ".cairn/chain.json"] {
        assert!(!is_link(p), "{p} is still a link");
    }
    assert!(is_link(".cairn/hotkeys.json") && is_link("config/out.json"));
    assert_eq!(read(&r.join("config/hotkeys.json")), "{\"new\":1}");
}

/// Every file and folder below `dir`, with the files' content.
fn snapshot(dir: &std::path::Path) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.to_string_lossy().into_owned();
        if p.is_dir() {
            out.push((name, None));
            out.extend(snapshot(&p));
        } else {
            out.push((name, Some(fs::read_to_string(&p).unwrap())));
        }
    }
    out.sort();
    out
}

/// A received vault can also link a whole config folder (`.cairn`, or
/// `.cairn/snippets` below it) out of the vault. Changing a setting then
/// fails with a plain error and writes nothing there, not even a folder;
/// reading the settings still works. A folder link that stays in the vault
/// is followed (FINDING-013, config folders).
#[cfg(unix)]
#[test]
fn config_folders_that_lead_out_of_the_vault_are_not_written() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    let o = outside.path();
    fs::create_dir_all(o.join("cairn")).unwrap();
    fs::write(o.join("cairn/settings.json"), "{\"theme\":\"dark\"}").unwrap();
    fs::create_dir_all(o.join("snippets")).unwrap();
    fs::write(o.join("snippets/wide.css"), "body{}").unwrap();
    let before = snapshot(o);
    let (d, v) = setup(&[("n.md", ""), ("config/x.json", "{}")]);
    let r = d.path();
    let out_err = |dir: &str| CoreError::Io(format!("The \"{dir}\" folder leads outside the vault."));

    // The whole .cairn folder links out.
    symlink(o.join("cairn"), r.join(".cairn")).unwrap();
    for name in ["settings.json", "workspace.json", "snippets/new.css", "plugins/p.js"] {
        assert_eq!(v.write_config(name, "{\"new\":1}"), Err(out_err(".cairn")), "{name}");
    }
    assert_eq!(v.read_config("settings.json").unwrap().as_deref(), Some("{\"theme\":\"dark\"}"));
    assert_eq!(snapshot(o), before);

    // A folder below it links out, or cannot be followed.
    fs::remove_file(r.join(".cairn")).unwrap();
    fs::create_dir(r.join(".cairn")).unwrap();
    symlink(o.join("snippets"), r.join(".cairn/snippets")).unwrap();
    symlink(o.join("missing"), r.join(".cairn/plugins")).unwrap();
    assert_eq!(v.write_config("snippets/new.css", "a{}"), Err(out_err(".cairn/snippets")));
    assert_eq!(v.write_config("snippets/wide.css", "a{}"), Err(out_err(".cairn/snippets")));
    assert_eq!(v.write_config("plugins/p.js", "x"), Err(out_err(".cairn/plugins")));
    assert_eq!(v.read_config("snippets/wide.css").unwrap().as_deref(), Some("body{}"));
    assert_eq!(v.list_config("snippets").unwrap(), vec!["wide.css"]);
    assert_eq!(snapshot(o), before);
    // The folders that are in the vault still take config files.
    v.write_config("settings.json", "{\"a\":1}").unwrap();
    assert_eq!(fs::read_to_string(r.join(".cairn/settings.json")).unwrap(), "{\"a\":1}");

    // A folder link that stays in the vault is followed.
    symlink("../config", r.join(".cairn/themes")).unwrap();
    v.write_config("themes/dark.css", "b{}").unwrap();
    assert_eq!(fs::read_to_string(r.join("config/dark.css")).unwrap(), "b{}");
    assert_eq!(snapshot(o), before);
}

#[test]
fn attachments_get_unique_names() {
    let (_d, v) = setup(&[]);
    let p1 = v.unique_path("attachments", "Pasted image", "png").unwrap();
    v.create_file(&p1, b"\x89PNG").unwrap();
    let p2 = v.unique_path("attachments", "Pasted image", "png").unwrap();
    assert_eq!(p1, "attachments/Pasted image.png");
    assert_eq!(p2, "attachments/Pasted image 1.png");
    assert_eq!(v.read_file(&p1).unwrap(), b"\x89PNG");
}

#[test]
fn write_file_with_base_hash_and_folder_helpers() {
    let (d, v) = setup(&[("a/b/pic.png", "one")]);
    let h1 = cairn_core::index::hash_hex(&cairn_core::index::hash_bytes(b"one"));
    v.write_file("a/b/pic.png", b"two", Some(&h1)).unwrap();
    assert!(matches!(v.write_file("a/b/pic.png", b"three", Some(&h1)), Err(CoreError::Conflict(_))));
    let ch = v.write_file("new/dir/n.md", b"# hi", None).unwrap().changes;
    assert_eq!(ch.len(), 3);
    assert_eq!(v.search("hi", 5)[0].path, "new/dir/n.md");
    v.ensure_folder("x/y").unwrap();
    v.ensure_folder("x/y").unwrap();
    assert!(d.path().join("x/y").is_dir());
    let ch = v.prune_empty_folders("x/y").unwrap();
    assert_eq!(ch.len(), 2);
    assert!(!d.path().join("x").exists());
    // a non-empty folder stops pruning
    assert!(v.prune_empty_folders("a/b").unwrap().is_empty());
}

#[test]
fn watcher_hint_finds_a_new_note_with_an_nfd_name() {
    let (d, v) = setup(&[]);
    fs::write(d.path().join("cafe\u{301}.md"), "from a mac").unwrap();
    let mapper = StdFs::new(d.path(), TrashMode::Vault).unwrap();
    let hint = mapper.to_vault_path(&d.path().canonicalize().unwrap().join("cafe\u{301}.md")).unwrap();
    let c = v.rescan_paths(&[hint]).unwrap();
    assert!(matches!(&c[..], [Change::Created { entry }] if entry.path == "caf\u{e9}.md"), "{c:?}");
    assert_eq!(v.search("mac", 5)[0].path, "caf\u{e9}.md");
}

/// A watcher event for a Unicode twin, which the mapping cannot name by
/// itself, rescans its folder. A name with a backslash has no vault path
/// and stays out.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn watcher_hints_find_unicode_twins() {
    let (d, v) = setup(&[("a/caf\u{e9}.md", "nfc")]);
    let root = d.path().canonicalize().unwrap();
    let mapper = StdFs::new(&root, TrashMode::Vault).unwrap();
    let hints = |names: &[&str]| -> Vec<String> { names.iter().filter_map(|n| mapper.to_vault_path(&root.join(n))).collect() };
    fs::write(root.join("a/cafe\u{301}.md"), "nfd twin").unwrap();
    fs::write(root.join("a/back\\slash.md"), "backslash").unwrap();
    assert!(hints(&["a/back\\slash.md"]).is_empty());
    let c = v.rescan_paths(&hints(&["a/cafe\u{301}.md", "a/back\\slash.md"])).unwrap();
    assert!(matches!(&c[..], [Change::Created { entry }] if entry.path == "a/caf\u{e9} (Unicode twin).md"), "{c:?}");
    fs::write(root.join("a/cafe\u{301}.md"), "nfd twin, edited").unwrap();
    let c = v.rescan_paths(&hints(&["a/cafe\u{301}.md"])).unwrap();
    assert!(matches!(&c[..], [Change::Modified { entry }] if entry.path == "a/caf\u{e9} (Unicode twin).md"), "{c:?}");
    assert_eq!(v.read_note("a/caf\u{e9}.md").unwrap().content, "nfc");
}

/// Names with a backslash stay out of the vault (by design), and the vault
/// says which ones, so that sync can list them as not synced.
#[cfg(unix)]
#[test]
fn names_with_a_backslash_are_reported_as_skipped() {
    let (d, v) = setup(&[("a/ok.md", "ok"), ("a/back\\slash.md", "b")]);
    assert_eq!(paths(&v), ["a", "a/ok.md"]);
    assert_eq!(v.skipped_backslash_names(), ["a/back\\slash.md"]);
    fs::create_dir(d.path().join("x\\y")).unwrap();
    fs::write(d.path().join("x\\y/n.md"), "n").unwrap();
    assert!(v.rescan().unwrap().is_empty());
    assert_eq!(v.skipped_backslash_names(), ["a/back\\slash.md", "x\\y"]);
    // Renamed outside Cairn, the note comes in and leaves the list.
    fs::rename(d.path().join("a/back\\slash.md"), d.path().join("a/back slash.md")).unwrap();
    let c = v.rescan().unwrap();
    assert!(matches!(&c[..], [Change::Created { entry }] if entry.path == "a/back slash.md"), "{c:?}");
    assert_eq!(v.skipped_backslash_names(), ["x\\y"]);
}

#[cfg(unix)]
#[test]
fn check_in_vault_refuses_paths_a_symlink_leads_out() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(outside.path().join("dir")).unwrap();
    fs::write(outside.path().join("dir/o.md"), "outside").unwrap();
    let (d, v) = setup(&[("a/n.md", "in")]);
    let r = d.path();
    symlink(outside.path().join("dir"), r.join("ext")).unwrap();
    symlink(outside.path().join("dir/o.md"), r.join("o.md")).unwrap();
    symlink(outside.path().join("gone.md"), r.join("dangling.md")).unwrap();
    symlink("a", r.join("alias")).unwrap();
    symlink("a/n.md", r.join("in.md")).unwrap();
    for p in ["a/n.md", "alias/n.md", "in.md", "new.md", "a/new/deeper.md", "alias/new.md"] {
        assert_eq!(v.check_in_vault(p), Ok(()), "{p}");
    }
    for p in ["ext", "ext/o.md", "ext/new.md", "ext/new/deeper.md", "o.md", "dangling.md"] {
        assert!(matches!(v.check_in_vault(p), Err(CoreError::InvalidPath(_))), "{p}");
    }
    // The app itself follows the links.
    assert_eq!(v.read_note("ext/o.md").unwrap().content, "outside");
}

/// A received vault (git clone, archive) can carry links to its own hidden
/// or non-Markdown files. Since a save follows a symlinked note, a plugin
/// writing such a "note" would rewrite the file it leads to: .git/config
/// (core.fsmonitor runs a command on the next `git status`), settings,
/// other plugins, data files.
#[cfg(unix)]
#[test]
fn check_in_vault_refuses_links_to_hidden_files_and_files_that_are_not_notes() {
    use std::os::unix::fs::symlink;
    let (d, v) = setup(&[
        (".git/config", "[core]\n"),
        (".cairn/settings.json", "{}"),
        (".trash/old.md", "old"),
        ("a/data.json", "{}"),
        ("a/n.md", "in"),
    ]);
    let r = d.path();
    fs::create_dir(r.join("notes")).unwrap();
    symlink("../.git/config", r.join("notes/setup.md")).unwrap();
    symlink("../.cairn/settings.json", r.join("notes/settings.md")).unwrap();
    symlink("../.trash/old.md", r.join("notes/old.md")).unwrap();
    symlink("../a/data.json", r.join("notes/data.md")).unwrap();
    symlink("../.git", r.join("notes/git")).unwrap();
    symlink("../a/n.md", r.join("notes/n.md")).unwrap();
    symlink("../a", r.join("notes/a")).unwrap();
    for p in ["notes/setup.md", "notes/settings.md", "notes/old.md", "notes/data.md", "notes/git", "notes/git/config", "notes/git/new.md"] {
        assert!(matches!(v.check_in_vault(p), Err(CoreError::InvalidPath(_))), "{p}");
    }
    // Links to notes and visible folders of the vault are fine.
    for p in ["notes/n.md", "notes/a", "notes/a/n.md", "notes/a/new.md"] {
        assert_eq!(v.check_in_vault(p), Ok(()), "{p}");
    }
}

#[test]
fn write_new_file_and_delete_file_check_the_disk() {
    let (d, v) = setup(&[("n.md", "one")]);
    // a file the index does not know yet is not replaced either
    fs::write(d.path().join("new.md"), "theirs").unwrap();
    assert!(matches!(v.write_new_file("new.md", b"ours"), Err(CoreError::AlreadyExists(_))));
    assert_eq!(fs::read_to_string(d.path().join("new.md")).unwrap(), "theirs");
    // any name the file system takes, unlike create_file
    assert!(matches!(v.create_file("a/Meeting #1.md", b"x"), Err(CoreError::InvalidName(_))));
    assert_eq!(v.write_new_file("a/Meeting #1.md", b"x").unwrap().changes.len(), 2);
    assert_eq!(v.search("x", 5)[0].path, "a/Meeting #1.md");
    // a delete only takes the content it was given
    let h1 = cairn_core::index::hash_hex(&cairn_core::index::hash_bytes(b"one"));
    fs::write(d.path().join("n.md"), "two").unwrap();
    assert!(matches!(v.delete_file("n.md", &h1), Err(CoreError::Conflict(_))));
    assert!(d.path().join("n.md").exists());
    fs::write(d.path().join("n.md"), "one").unwrap();
    assert_eq!(v.delete_file("n.md", &h1).unwrap(), vec![Change::Deleted { path: "n.md".into(), kind: EntryKind::File }]);
    assert!(!d.path().join("n.md").exists() && d.path().join(".trash/n.md").exists());
    assert!(!paths(&v).contains(&"n.md".to_string()));
    assert!(matches!(v.delete_file("n.md", &h1), Err(CoreError::NotFound(_))));
}

/// Sync writes files from other devices with write_new_file. The checks
/// create_file makes for the user (FINDING-053: no case twin, no new folder
/// with a name the app would not make) do not apply there: a case-sensitive
/// device keeps `Note.md` next to `note.md`, as the other device has them.
#[cfg(target_os = "linux")]
#[test]
fn write_new_file_takes_a_case_twin_and_any_folder_name() {
    let (d, v) = setup(&[("note.md", "lower")]);
    assert!(matches!(v.create_file("Note.md", b"upper"), Err(CoreError::AlreadyExists(_))));
    v.write_new_file("Note.md", b"upper").unwrap();
    assert_eq!(fs::read_to_string(d.path().join("Note.md")).unwrap(), "upper");
    assert_eq!(fs::read_to_string(d.path().join("note.md")).unwrap(), "lower");
    assert!(matches!(v.create_file("Meeting #1/n.md", b"x"), Err(CoreError::InvalidName(_))));
    v.write_new_file("Meeting #1/n.md", b"x").unwrap();
    assert_eq!(paths(&v), ["Meeting #1", "Meeting #1/n.md", "Note.md", "note.md"]);
}
