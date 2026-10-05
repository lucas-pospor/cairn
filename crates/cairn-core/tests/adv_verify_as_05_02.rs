//! Not a defect (by design): delete_entry through a symlinked folder removes
//! a file outside the vault. Question: is anything ever lost
//! for good, on any path the desktop app can take?
//!
//! The app opens vaults with `StdFs::new(path, TrashMode::System)`
//! (`vault_fs` in app/src-tauri/src/commands.rs). `StdFs::remove` tries the
//! OS trash and falls back to `<vault>/.trash` (`move_to_vault_trash`). These
//! tests cover what adv_verify_as_05.rs does not: the round trip back out of the
//! OS trash, the fallback when the OS trash is unusable, the fallback across
//! file systems, and a whole folder inside the linked folder.
//!
//! The freedesktop home trash is redirected with XDG_DATA_HOME into each
//! test's temp dir (same file system as the vault), so the real trash is
//! never touched.
//!
//! Run with:
//!   cargo test -p cairn-core --test adv_verify_as_05_02 -- --ignored --nocapture --test-threads=1

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cairn_core::{StdFs, TrashMode, Vault};

static ENV: Mutex<()> = Mutex::new(());

struct Setup {
    _tmp: tempfile::TempDir,
    _out_tmp: Option<tempfile::TempDir>,
    vault_dir: PathBuf,
    outside: PathBuf,
    data: PathBuf,
}

/// `outside_base`: where the linked folder's real target lives. `None` puts it
/// in the same temp dir (same file system) as the vault.
fn setup(outside_base: Option<&Path>) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (out_tmp, out_base) = match outside_base {
        Some(b) => {
            let t = tempfile::Builder::new().prefix("cairn-as05-").tempdir_in(b).unwrap();
            let p = t.path().canonicalize().unwrap();
            (Some(t), p)
        }
        None => (None, base.clone()),
    };
    let vault_dir = base.join("vault");
    let outside = out_base.join("outside").join("dir");
    let data = base.join("data");
    fs::create_dir_all(&vault_dir).unwrap();
    fs::create_dir_all(outside.join("sub")).unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::write(vault_dir.join("Welcome.md"), "# hi\n").unwrap();
    fs::write(outside.join("victim.md"), "please-keep-me\n").unwrap();
    fs::write(outside.join("keep.md"), "keep\n").unwrap();
    fs::write(outside.join("sub").join("a.md"), "sub-a\n").unwrap();
    fs::write(outside.join("sub").join("b.png"), [0x89u8, b'P', b'N', b'G']).unwrap();
    symlink(&outside, vault_dir.join("linked")).unwrap();
    // SAFETY: every test holds ENV and the file runs with --test-threads=1.
    unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
    Setup { _tmp: tmp, _out_tmp: out_tmp, vault_dir, outside, data }
}

fn open(s: &Setup, mode: TrashMode) -> Vault {
    Vault::open(Arc::new(StdFs::new(&s.vault_dir, mode).unwrap())).unwrap()
}

#[test]
#[ignore = "not a defect (by design), impact: the OS-trash copy restores to the real outside path (passes)"]
fn os_trash_round_trip_restores_the_outside_file() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let s = setup(None);
    let v = open(&s, TrashMode::System);
    v.delete("linked/victim.md").unwrap();
    let orig = s.outside.join("victim.md");
    println!("after delete, outside file present: {}", orig.exists());
    assert!(!orig.exists(), "symptom: the file left its location");

    // What a file manager's "Restore" does, via the same crate the app uses.
    let items: Vec<_> = trash::os_limited::list()
        .unwrap()
        .into_iter()
        .filter(|i| i.original_parent.starts_with(&s.outside))
        .collect();
    println!("trash items for the outside folder: {:?}", items.iter().map(|i| i.original_path()).collect::<Vec<_>>());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].original_path(), orig);
    trash::os_limited::restore_all(items).unwrap();
    println!("after restore, outside file: {:?}", fs::read_to_string(&orig).ok());
    assert_eq!(fs::read_to_string(&orig).unwrap(), "please-keep-me\n");
    assert!(!s.vault_dir.join(".trash").exists());
}

#[test]
#[ignore = "not a defect (by design), impact: OS trash unusable -> falls back to <vault>/.trash, file intact (passes)"]
fn unusable_os_trash_falls_back_to_vault_trash_intact() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let s = setup(None);
    // Make the home trash impossible to create.
    fs::set_permissions(&s.data, fs::Permissions::from_mode(0o500)).unwrap();
    let v = open(&s, TrashMode::System);
    let r = v.delete("linked/victim.md");
    fs::set_permissions(&s.data, fs::Permissions::from_mode(0o700)).unwrap();
    println!("delete result: {r:?}");
    r.unwrap();
    let in_vault_trash = s.vault_dir.join(".trash").join("victim.md");
    println!("outside present: {}  vault-trash copy: {:?}", s.outside.join("victim.md").exists(), fs::read_to_string(&in_vault_trash).ok());
    assert!(!s.data.join("Trash").join("files").join("victim.md").exists());
    assert_eq!(fs::read_to_string(&in_vault_trash).unwrap(), "please-keep-me\n");
    assert!(s.outside.join("keep.md").exists());
}

#[test]
#[ignore = "not a defect (by design), impact: vault-trash fallback across file systems refuses (EXDEV) and keeps the file (passes)"]
fn vault_trash_fallback_across_file_systems_keeps_the_file() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // /dev/shm and /tmp are separate tmpfs mounts on this machine.
    let shm = Path::new("/dev/shm");
    if !shm.is_dir() {
        println!("no /dev/shm, skipping");
        return;
    }
    let s = setup(Some(shm));
    // TrashMode::Vault runs exactly the fallback branch of TrashMode::System.
    let v = open(&s, TrashMode::Vault);
    let r = v.delete("linked/victim.md");
    println!("delete result: {r:?}");
    println!("outside present: {}", s.outside.join("victim.md").exists());
    assert!(r.is_err(), "rename across file systems fails");
    assert_eq!(fs::read_to_string(s.outside.join("victim.md")).unwrap(), "please-keep-me\n");
    // The index still lists it, because the delete failed before remove_tree.
    assert!(v.entries().iter().any(|e| e.path == "linked/victim.md"));
}

#[test]
#[ignore = "not a defect (by design), impact: a folder inside the linked folder goes to the OS trash whole (passes)"]
fn folder_inside_linked_folder_goes_to_os_trash_whole() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let s = setup(None);
    let v = open(&s, TrashMode::System);
    v.delete("linked/sub").unwrap();
    let t = s.data.join("Trash").join("files").join("sub");
    let info = fs::read_to_string(s.data.join("Trash").join("info").join("sub.trashinfo")).unwrap();
    println!("outside sub present: {}\ntrashed: a.md={:?} b.png={}\n{info}", s.outside.join("sub").exists(), fs::read_to_string(t.join("a.md")).ok(), t.join("b.png").exists());
    assert!(!s.outside.join("sub").exists());
    assert_eq!(fs::read_to_string(t.join("a.md")).unwrap(), "sub-a\n");
    assert!(t.join("b.png").exists());
    assert!(info.contains(&format!("Path={}", s.outside.join("sub").display())));
    assert!(s.outside.join("victim.md").exists());
}

#[test]
#[ignore = "not a defect (by design), context: deleting the link itself never touches the target, in every mode (passes)"]
fn deleting_the_link_itself_never_touches_the_target() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for mode in [TrashMode::Vault, TrashMode::Permanent] {
        let s = setup(None);
        let v = open(&s, mode);
        v.delete("linked").unwrap();
        println!("{mode:?}: link gone={} target files: victim={} keep={} sub/a={}", !s.vault_dir.join("linked").exists(), s.outside.join("victim.md").exists(), s.outside.join("keep.md").exists(), s.outside.join("sub/a.md").exists());
        assert!(fs::symlink_metadata(s.vault_dir.join("linked")).is_err());
        assert!(s.outside.join("victim.md").exists());
        assert!(s.outside.join("keep.md").exists());
        assert!(s.outside.join("sub/a.md").exists());
    }
}
