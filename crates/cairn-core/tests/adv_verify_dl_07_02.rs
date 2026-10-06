//! Regression tests for FINDING-046 (the scanner trusts size +
//! whole-millisecond mtime; `write_note` with a base hash checks the bytes
//! on disk).
//!
//! Two questions that decide how bad this is:
//!
//! 1. `dl07_02_mtime_preserving_copy_on_fine_fs_is_detected` (control):
//!    rsync -t / cp -p / tar / unzip are the usual triggers. On a normal
//!    file system those tools copy the *source's* mtime, which is not Cairn's
//!    last-save mtime unless the source was written in the same millisecond.
//!    A cp -p style restore of a copy edited elsewhere is noticed and the
//!    stale autosave gets a conflict. Only a forged mtime (utimes) or a
//!    coarse-mtime file system gets past the scanner.
//!
//! 2. `dl07_02_later_edit_does_not_revert_an_invisible_collision`
//!    (regression test): on a 1 s mtime file system (sshfs/SFTP, HFS+,
//!    simulated by truncating StdFs mtimes, no utimes calls) a same-size
//!    external edit that lands in the same second as an autosave stays
//!    invisible after the second has passed: a full rescan sees nothing and
//!    search keeps the old text. An edit made later in the still-open tab
//!    must get a conflict instead of silently reverting it.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_dl_07_02 -- --nocapture --test-threads=1

use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cairn_core::{CoreError, FileStat, StdFs, TrashMode, Vault, VaultFs};

/// StdFs with mtimes truncated to `gran_ms` (1 s for SFTP/sshfs and HFS+).
struct CoarseFs {
    inner: StdFs,
    gran_ms: i64,
}

impl CoarseFs {
    fn fix(&self, mut s: FileStat) -> FileStat {
        s.mtime -= s.mtime.rem_euclid(self.gran_ms);
        s
    }
}

impl VaultFs for CoarseFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        Ok(self.inner.list(dir)?.into_iter().map(|s| self.fix(s)).collect())
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        Ok(self.inner.stat(path)?.map(|s| self.fix(s)))
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.inner.read(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        Ok(self.fix(self.inner.write(path, data)?))
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.remove(path)
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.inner.remove_empty_dir(path)
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

#[test]
fn dl07_02_mtime_preserving_copy_on_fine_fs_is_detected() {
    let vault_dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(vault_dir.path().join("n.md"), "buy milk\n").unwrap();
    let v = Vault::open(Arc::new(StdFs::new(vault_dir.path(), TrashMode::Vault).unwrap())).unwrap();
    let opened = v.read_note("n.md").unwrap();
    // cp -p n.md elsewhere/n.md (a backup / the other machine's copy).
    let src = elsewhere.path().join("n.md");
    fs::copy(vault_dir.path().join("n.md"), &src).unwrap();
    // Cairn autosaves the user's typing.
    let saved = v.write_note("n.md", "buy milk\nx", Some(&opened.hash)).unwrap();
    // The other copy is edited at human speed (same size as Cairn's version).
    std::thread::sleep(Duration::from_millis(50));
    fs::write(&src, "buy eggs\nx").unwrap();
    // rsync -t / cp -p back into the vault: content and the source's mtime.
    let src_mtime = fs::metadata(&src).unwrap().modified().unwrap();
    fs::copy(&src, vault_dir.path().join("n.md")).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(vault_dir.path().join("n.md"))
        .unwrap()
        .set_modified(src_mtime)
        .unwrap();
    let seen = v.rescan_paths(&["n.md".to_string()]).unwrap();
    let w = v.write_note("n.md", "buy milk\nxy", Some(&saved.hash));
    let disk = fs::read_to_string(vault_dir.path().join("n.md")).unwrap();
    println!("rescan saw: {seen:?}\nwrite: {:?}\ndisk: {disk:?}", w.as_ref().map(|_| "Ok"));
    assert!(!seen.is_empty(), "mtime-preserving copy was not noticed by the scanner");
    assert!(matches!(w, Err(CoreError::Conflict(_))), "stale autosave was accepted");
    assert_eq!(disk, "buy eggs\nx");
}

#[test]
fn dl07_02_later_edit_does_not_revert_an_invisible_collision() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Shopping.md"), "- [ ] buy milk\n- [ ] call bob\n").unwrap();
    let v = Vault::open(Arc::new(CoarseFs { inner: StdFs::new(d.path(), TrashMode::Vault).unwrap(), gran_ms: 1000 }))
        .unwrap();
    let opened = v.read_note("Shopping.md").unwrap();

    while now_ms().rem_euclid(1000) > 100 {
        std::thread::sleep(Duration::from_millis(5));
    }
    let saved = v
        .write_note("Shopping.md", "- [ ] buy milk\n- [ ] call bob\nnote: ", Some(&opened.hash))
        .unwrap();
    // 200 ms later another program writes a same-size change (plain write).
    std::thread::sleep(Duration::from_millis(200));
    fs::write(d.path().join("Shopping.md"), "- [ ] buy eggs\n- [ ] call bob\nnote: ").unwrap();

    // Long after that second has passed, nothing has noticed the change.
    std::thread::sleep(Duration::from_millis(2500));
    let full_rescan = v.rescan().unwrap();
    let hits_eggs = v.search("eggs", 10).len();
    let hits_milk = v.search("milk", 10).len();

    // The user types in the still-open tab; autosave uses the old base hash.
    let w = v.write_note("Shopping.md", "- [ ] buy milk\n- [ ] call bob\nnote: later", Some(&saved.hash));
    let disk = fs::read_to_string(d.path().join("Shopping.md")).unwrap();
    println!(
        "full rescan 2.5 s later: {full_rescan:?}\nsearch eggs={hits_eggs} milk={hits_milk}\nwrite: {:?}\ndisk: {disk:?}",
        w.as_ref().map(|_| "Ok")
    );
    assert!(
        matches!(w, Err(CoreError::Conflict(_))) || disk.contains("eggs"),
        "external edit silently reverted by a later autosave (rescan saw {full_rescan:?}, search eggs={hits_eggs})"
    );
}
