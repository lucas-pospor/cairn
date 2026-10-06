//! Adversarial tests: file names created outside Cairn: Unicode
//! normalization, singleton decompositions, backslashes, non-UTF-8 bytes.
//!
//! Tests named `fsNN_*` are regression tests for fixed findings.
//! Run one with:
//!   cargo test -p cairn-core --test adv_fs_names -- --exact <name>
//! Run them all with:
//!   cargo test -p cairn-core --test adv_fs_names

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{Change, StdFs, TrashMode, Vault};

const NFC: &str = "caf\u{e9}.md"; // "café.md", precomposed
const NFD: &str = "cafe\u{301}.md"; // "café.md", e + combining acute (macOS style)
const TWIN: &str = "caf\u{e9} (Unicode twin).md"; // how NFD is listed next to NFC

fn open(p: &Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap())).unwrap()
}

fn disk_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

fn entries(v: &Vault) -> Vec<String> {
    let mut p: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    p.sort();
    p
}

/// A vault with one note whose name is NFD on disk (as written by macOS,
/// a Mac-made zip, rsync/Syncthing from a Mac, git without precomposeunicode).
fn nfd_vault() -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(NFD), "hello from a mac").unwrap();
    let v = open(d.path());
    (d, v)
}

// ---------------------------------------------------------------------------
// FINDING-011: names that are not NFC on disk
// ---------------------------------------------------------------------------

#[test]
fn fs02_nfd_note_can_be_read() {
    let (_d, v) = nfd_vault();
    assert_eq!(entries(&v), vec![NFC]); // listed under its NFC identity
    let r = v.read_note(NFC);
    assert!(r.is_ok(), "read_note: {:?}", r.err());
    assert_eq!(v.search("hello", 10).len(), 1, "content was never indexed");
}

#[test]
fn fs02_nfd_folder_contents_are_listed() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("Re\u{301}sume\u{301}")).unwrap();
    fs::write(d.path().join("Re\u{301}sume\u{301}/inside.md"), "in folder").unwrap();
    let v = open(d.path());
    assert_eq!(entries(&v), vec!["R\u{e9}sum\u{e9}".to_string(), "R\u{e9}sum\u{e9}/inside.md".to_string()]);
}

#[test]
fn fs02_saving_nfd_note_creates_no_duplicate_file() {
    let (d, v) = nfd_vault();
    let w = v.write_note(NFC, "edited in cairn", None);
    assert!(w.is_ok());
    assert_eq!(disk_names(d.path()).len(), 1, "files on disk: {:?}", disk_names(d.path()).iter().map(|n| n.escape_unicode().to_string()).collect::<Vec<_>>());
}

#[test]
fn fs02_nfd_note_can_be_renamed_and_deleted() {
    let (d, v) = nfd_vault();
    let r = v.rename(NFC, "other.md");
    assert!(r.is_ok(), "rename: {r:?}");
    assert!(d.path().join("other.md").exists());
    let (_d2, v2) = nfd_vault();
    let r = v2.delete(NFC);
    assert!(r.is_ok(), "delete: {r:?}");
}

#[test]
fn fs02_watcher_hint_reports_no_phantom_changes() {
    let (d, v) = nfd_vault();
    // The watcher maps the OS path through StdFs::to_vault_path (NFC).
    let mapper = StdFs::new(d.path(), TrashMode::Vault).unwrap();
    let hint = mapper.to_vault_path(&d.path().canonicalize().unwrap().join(NFD)).unwrap();
    let a = v.rescan_paths(&[hint]).unwrap();
    let b = v.rescan().unwrap();
    assert!(a.is_empty() && b.is_empty(), "rescan_paths: {a:?}\nrescan: {b:?}");
}

#[test]
fn fs02_nfc_nfd_twins_do_not_resurrect_after_delete() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(NFC), "nfc twin").unwrap();
    fs::write(d.path().join(NFD), "nfd twin").unwrap();
    let v = open(d.path());
    // Two files, two entries: the NFD one under a twin name.
    assert_eq!(entries(&v), vec![TWIN, NFC]);
    assert_eq!(v.read_note(TWIN).unwrap().content, "nfd twin");
    v.delete(NFC).unwrap();
    let c = v.rescan().unwrap();
    assert!(!c.iter().any(|c| matches!(c, Change::Created { .. })), "deleted note came back: {c:?}");
    // The twin, listed all along, now has the name to itself.
    assert!(matches!(&c[..], [Change::Renamed { from, entry }] if from == TWIN && entry.path == NFC), "{c:?}");
    assert_eq!(v.read_note(NFC).unwrap().content, "nfd twin");
}

#[cfg(not(target_os = "macos"))] // twins cannot exist there
#[test]
fn a_twin_is_renamed_and_deleted_on_its_own() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(NFC), "nfc twin").unwrap();
    fs::write(d.path().join(NFD), "nfd twin").unwrap();
    let v = open(d.path());
    v.rename(TWIN, "moved.md").unwrap();
    assert_eq!(disk_names(d.path()), vec![NFC, "moved.md"]);
    v.rename("moved.md", TWIN).unwrap(); // now a plain file with that name
    v.delete(TWIN).unwrap();
    assert!(v.rescan().unwrap().is_empty());
    assert_eq!(entries(&v), vec![NFC]);
    assert_eq!(disk_names(d.path()), vec![NFC]);
    assert_eq!(v.read_note(NFC).unwrap().content, "nfc twin");
}

#[test]
fn fs02_singleton_decomposition_names_can_be_read() {
    let d = tempfile::tempdir().unwrap();
    // Each of these changes under NFC even though it is a single code point.
    let names = ["\u{212a}elvin.md", "10 \u{2126}.md", "\u{f91d}.md"];
    for n in names {
        fs::write(d.path().join(n), "x").unwrap();
    }
    let v = open(d.path());
    let mut failed = Vec::new();
    for e in v.entries() {
        if let Err(err) = v.read_note(&e.path) {
            failed.push(format!("{} -> {err}", e.path.escape_unicode()));
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

#[test]
fn nfd_input_from_the_ui_is_stored_as_nfc() {
    let d = tempfile::tempdir().unwrap();
    let v = open(d.path());
    let r = v.create_note(NFD, "x").unwrap();
    assert_eq!(r.entry.path, NFC);
    assert_eq!(disk_names(d.path()), vec![NFC]);
    assert_eq!(v.read_note(NFD).unwrap().content, "x");
    assert_eq!(v.read_note(NFC).unwrap().content, "x");
    assert!(matches!(v.create_note(NFC, "y"), Err(cairn_core::CoreError::AlreadyExists(_))));
    assert!(v.rescan().unwrap().is_empty());
}

#[test]
fn precomposed_emoji_and_zero_width_names_round_trip() {
    let d = tempfile::tempdir().unwrap();
    let v = open(d.path());
    for n in ["🪨 cairn.md", "zero\u{200b}width.md", "日本語.md", "Ünïcödé.md", "👩‍👩‍👧 family.md"] {
        let r = v.create_note(n, n).unwrap();
        assert_eq!(v.read_note(&r.entry.path).unwrap().content, n);
        v.rename(&r.entry.path, &format!("moved {n}")).unwrap();
        v.delete(&format!("moved {n}")).unwrap();
    }
    assert!(v.rescan().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// FINDING-138: backslashes in names created outside Cairn (Linux)
// ---------------------------------------------------------------------------

#[test]
fn fs18_backslash_names_do_not_map_to_wrong_paths() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("back\\slash.md"), "b").unwrap();
    // On Linux a backslash is an ordinary name character: a file named `\`.
    #[allow(clippy::join_absolute_paths)]
    fs::write(d.path().join("\\"), "root?").unwrap();
    fs::write(d.path().join("ok.md"), "ok").unwrap();
    let v = open(d.path());
    let listed = entries(&v);
    let mut problems = Vec::new();
    for e in v.entries() {
        if e.path.is_empty() {
            problems.push("entry with empty path (the vault root)".to_string());
        } else if e.kind == cairn_core::EntryKind::File && v.read_file(&e.path).is_err() {
            problems.push(format!("listed but unreadable: {}", e.path));
        }
    }
    assert!(problems.is_empty(), "listed {listed:?}; problems {problems:?}");
}

/// `a`, then `bad`, then `b`, as a name that is not Unicode: `bad` is a byte
/// that is not UTF-8 on Unix, and an unpaired surrogate on Windows, where
/// names are UTF-16.
fn not_unicode(a: &str, bad: u8, b: &str) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec([a.as_bytes(), &[bad], b.as_bytes()].concat())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let wide: Vec<u16> = a.encode_utf16().chain([0xd800 | u16::from(bad)]).chain(b.encode_utf16()).collect();
        std::ffi::OsString::from_wide(&wide)
    }
}

#[test]
fn non_utf8_file_names_are_skipped_without_breaking_open() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(not_unicode("bad", 0xff, "name.md")), "x").unwrap();
    fs::create_dir(d.path().join(not_unicode("dir", 0xfe, ""))).unwrap();
    fs::write(d.path().join("good.md"), "g").unwrap();
    let v = open(d.path());
    assert_eq!(entries(&v), vec!["good.md"]);
    assert!(v.rescan().unwrap().is_empty());
}
