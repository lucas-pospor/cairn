//! Reproduction for FINDING-051: a save temp file with a predictable name
//! (`.<name>.cairn-tmp-<pid>`) opened with `File::create` (no O_EXCL, follows
//! symlinks) would let a pre-planted symlink at that path redirect the write.
//! `StdFs` creates a random temp name with O_EXCL (`create_new_file`).
//!
//! These tests go a little further than fs08 in adv_fs_ops.rs:
//! - a dangling symlink must not make Cairn CREATE a new file outside the vault,
//! - a `set_permissions` call that followed the symlink would change the
//!   outside file's mode to the note's mode,
//! - `write_config` (`.cairn/settings.json`) goes through the same path,
//! - a symlink "spray" over a PID range is invisible to Cairn (hidden files are
//!   skipped by the scanner), so with a PID-based name the attacker would not
//!   need the exact PID, only a range that contains it. A local user who can
//!   write the vault folder can read the PID from /proc and needs no guess.
//!
//! Run: cargo test -p cairn-core --test adv_verify_fs_08 -- --ignored --nocapture

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use cairn_core::{StdFs, TrashMode, Vault};

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

fn mode(p: &Path) -> u32 {
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[test]
fn fs08_dangling_temp_symlink_creates_file_outside_vault() {
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("autostart").join("evil.desktop");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    let (d, v) = setup(&[("note.md", "n")]);
    symlink(&target, d.path().join(format!(".note.md.cairn-tmp-{}", std::process::id()))).unwrap();
    let n = v.read_note("note.md").unwrap();
    let r = v.write_note("note.md", "[Desktop Entry]\nExec=sh -c 'echo pwned'\n", Some(&n.hash));
    println!("write result ok: {}", r.is_ok());
    println!("outside file exists: {}", target.exists());
    if target.exists() {
        println!("outside file content: {:?}", fs::read_to_string(&target).unwrap());
    }
    println!(
        "note.md is symlink: {}",
        fs::symlink_metadata(d.path().join("note.md")).unwrap().file_type().is_symlink()
    );
    assert!(!target.exists(), "save created a new file outside the vault");
}

#[test]
fn fs08_temp_symlink_changes_mode_of_outside_file() {
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("id_secret");
    fs::write(&victim, "secret key").unwrap();
    fs::set_permissions(&victim, fs::Permissions::from_mode(0o600)).unwrap();
    let (d, v) = setup(&[("note.md", "n")]);
    fs::set_permissions(d.path().join("note.md"), fs::Permissions::from_mode(0o644)).unwrap();
    symlink(&victim, d.path().join(format!(".note.md.cairn-tmp-{}", std::process::id()))).unwrap();
    let n = v.read_note("note.md").unwrap();
    let _ = v.write_note("note.md", "edited", Some(&n.hash));
    let m = mode(&victim);
    println!("outside file mode after save: {m:o} (was 600)");
    println!("outside file content after save: {:?}", fs::read_to_string(&victim).unwrap());
    assert_eq!(m, 0o600, "save changed the permissions of a file outside the vault");
}

#[test]
fn fs08_write_config_follows_temp_symlink() {
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, "victim original").unwrap();
    let (d, v) = setup(&[("note.md", "n"), (".cairn/settings.json", "{}")]);
    symlink(&victim, d.path().join(format!(".cairn/.settings.json.cairn-tmp-{}", std::process::id()))).unwrap();
    let r = v.write_config("settings.json", "{\"theme\":\"dark\"}");
    let now = fs::read_to_string(&victim).unwrap();
    println!("write_config ok: {} ; outside file now: {now:?}", r.is_ok());
    assert_eq!(now, "victim original", "settings save overwrote a file outside the vault");
}

#[test]
fn fs08_symlink_spray_over_pid_range() {
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, "victim original").unwrap();
    let (d, v) = setup(&[("Welcome.md", "# Welcome\n")]);
    let pid = std::process::id();
    let lo = pid.saturating_sub(2000).max(1);
    let hi = pid + 2000;
    for p in lo..=hi {
        symlink(&victim, d.path().join(format!(".Welcome.md.cairn-tmp-{p}"))).unwrap();
    }
    // Reopen: the planted links must not show up anywhere in the vault.
    let v2 = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    let visible: Vec<String> = v2.entries().into_iter().map(|e| e.path).collect();
    println!("planted {} symlinks; entries Cairn sees: {visible:?}", hi - lo + 1);
    drop(v);
    let n = v2.read_note("Welcome.md").unwrap();
    let _ = v2.write_note("Welcome.md", "# Welcome\nedited by the user\n", Some(&n.hash));
    let now = fs::read_to_string(&victim).unwrap();
    println!("outside file now: {now:?}");
    assert_eq!(now, "victim original", "sprayed symlink redirected the save outside the vault");
}
