//! Not a defect (by design): delete_entry through a symlinked folder removes
//! a file outside the vault. These tests check where the file goes, whether
//! it is recoverable, and whether deleting the linked folder itself touches
//! the target.
//!
//! The desktop app opens vaults with `StdFs::new(path, TrashMode::System)`
//! (`vault_fs` in app/src-tauri/src/commands.rs) and builds cairn-core with the
//! `system-trash` feature, so these tests use the same mode. The freedesktop
//! "home trash" is redirected with XDG_DATA_HOME to a temp folder on the same
//! file system as the vault, so nothing touches the real trash.
//!
//! Run with:
//!   cargo test -p cairn-core --test adv_verify_as_05 -- --ignored --nocapture --test-threads=1

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::{Arc, Mutex};

use cairn_core::{StdFs, TrashMode, Vault};

static ENV: Mutex<()> = Mutex::new(());

struct Setup {
    _tmp: tempfile::TempDir,
    vault_dir: std::path::PathBuf,
    outside: std::path::PathBuf,
    trash: std::path::PathBuf,
}

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let vault_dir = base.join("vault");
    let outside = base.join("outside").join("dir");
    let data = base.join("data");
    fs::create_dir_all(&vault_dir).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::write(vault_dir.join("Welcome.md"), "# hi\n").unwrap();
    fs::write(outside.join("victim.md"), "please-keep-me\n").unwrap();
    fs::write(outside.join("keep.md"), "keep\n").unwrap();
    symlink(&outside, vault_dir.join("linked")).unwrap();
    // SAFETY: tests in this file hold ENV while the env var matters and run
    // with --test-threads=1.
    unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
    Setup { _tmp: tmp, vault_dir, outside, trash: data.join("Trash") }
}

fn trashinfo_path(trash: &Path, name: &str) -> String {
    let info = fs::read_to_string(trash.join("info").join(format!("{name}.trashinfo"))).unwrap();
    info.lines().find_map(|l| l.strip_prefix("Path=")).unwrap_or("").to_string()
}

#[test]
#[ignore = "not a defect (by design): a delete through a linked folder moves the real outside file to the trash"]
fn delete_through_linked_folder_goes_to_system_trash_and_is_restorable() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let s = setup();
    let v = Vault::open(Arc::new(StdFs::new(&s.vault_dir, TrashMode::System).unwrap())).unwrap();

    // The linked folder is shown as part of the vault (`StdFs::stat_abs` follows it on purpose).
    let paths: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    println!("vault entries: {paths:?}");
    assert!(paths.iter().any(|p| p == "linked/victim.md"), "linked folder contents are listed");

    v.delete("linked/victim.md").unwrap();
    let outside_file = s.outside.join("victim.md");
    let in_trash = s.trash.join("files").join("victim.md");
    println!("outside file still at its place: {}", outside_file.exists());
    println!("in system trash: {} -> {:?}", in_trash.exists(), fs::read_to_string(&in_trash).ok());
    let orig = trashinfo_path(&s.trash, "victim.md");
    println!("trashinfo Path= {orig}");
    println!("vault/.trash exists: {}", s.vault_dir.join(".trash").exists());

    // The symptom: the file left its real location...
    assert!(!outside_file.exists());
    // ...but it is in the system trash, intact, with its real original path,
    // so "Restore" in the file manager puts it back where it was.
    assert_eq!(fs::read_to_string(&in_trash).unwrap(), "please-keep-me\n");
    assert_eq!(orig, outside_file.to_string_lossy());
    // The vault-trash fallback was not used.
    assert!(!s.vault_dir.join(".trash").exists());
    // Siblings are untouched.
    assert!(s.outside.join("keep.md").exists());
}

#[test]
#[ignore = "not a defect (by design), context: deleting the linked folder itself trashes only the link (passes)"]
fn delete_linked_folder_itself_trashes_only_the_symlink() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let s = setup();
    let v = Vault::open(Arc::new(StdFs::new(&s.vault_dir, TrashMode::System).unwrap())).unwrap();
    v.delete("linked").unwrap();
    let trashed = s.trash.join("files").join("linked");
    let md = fs::symlink_metadata(&trashed).unwrap();
    println!("trashed 'linked' is symlink: {}", md.file_type().is_symlink());
    println!("outside files still there: {} {}", s.outside.join("victim.md").exists(), s.outside.join("keep.md").exists());
    assert!(md.file_type().is_symlink());
    assert!(s.outside.join("victim.md").exists());
    assert!(s.outside.join("keep.md").exists());
    assert!(!s.vault_dir.join("linked").exists());
}

#[test]
#[ignore = "not a defect (by design), context: vault-trash mode (Android app storage) keeps the file in <vault>/.trash (passes)"]
fn vault_trash_mode_keeps_the_outside_file_in_vault_trash() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let s = setup();
    let v = Vault::open(Arc::new(StdFs::new(&s.vault_dir, TrashMode::Vault).unwrap())).unwrap();
    v.delete("linked/victim.md").unwrap();
    let in_vault_trash = s.vault_dir.join(".trash").join("victim.md");
    println!("outside file still at its place: {}", s.outside.join("victim.md").exists());
    println!("in vault trash: {:?}", fs::read_to_string(&in_vault_trash).ok());
    assert!(!s.outside.join("victim.md").exists());
    assert_eq!(fs::read_to_string(&in_vault_trash).unwrap(), "please-keep-me\n");
}
