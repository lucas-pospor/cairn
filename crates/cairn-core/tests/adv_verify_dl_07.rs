//! Reproduction for FINDING-046: the scanner trusts a file's size and mtime
//! (whole milliseconds), so a same-size external edit that lands in the same
//! mtime "bucket" is invisible to it. The next autosave must not overwrite
//! that edit: `write_note` with a base hash checks the bytes on disk
//! (`Vault::put`), not the indexed hash.
//!
//! fs10 in adv_fs_ops.rs forces the collision by setting the mtime. These
//! tests do not touch mtimes at all:
//!   * `dl07_coarse_mtime_fs_*` wraps `StdFs` so mtimes are truncated to 2 s,
//!     which is what the Linux vfat driver does on a FAT32 USB stick / SD card
//!     (1 s for ext3/HFS+/many NFS/SMB servers). The external program just
//!     writes 300 ms after Cairn's autosave.
//!   * `dl07_fine_grained_fs_natural_race_rate` measures, on tmpfs and on the
//!     real disk, how often an external same-size write issued right after
//!     Cairn's save collides at millisecond precision, and with a 2 ms gap.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_dl_07 -- --nocapture

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cairn_core::{CoreError, FileStat, StdFs, TrashMode, Vault, VaultFs};

/// StdFs with mtimes truncated to `gran_ms` (models FAT32's 2 s granularity).
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
fn dl07_coarse_mtime_fs_same_size_edit_is_not_overwritten() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("Tasks.md"), "- [ ] buy milk\n- [ ] call bob\n").unwrap();
    let v = Vault::open(Arc::new(CoarseFs { inner: StdFs::new(d.path(), TrashMode::Vault).unwrap(), gran_ms: 2000 })).unwrap();
    let opened = v.read_note("Tasks.md").unwrap();

    // Start the autosave early in a 2 s bucket so the external write 300 ms
    // later falls in the same bucket (on real FAT32 this is just luck: any
    // external save within the same 2 s window as Cairn's last autosave).
    while now_ms().rem_euclid(2000) > 200 {
        std::thread::sleep(Duration::from_millis(5));
    }
    // Cairn autosaves the user's typing.
    let saved = v
        .write_note("Tasks.md", "- [ ] buy milk\n- [ ] call bob\nnotes: ", Some(&opened.hash))
        .unwrap();
    // 300 ms later another program (phone sync, task app, script) ticks a
    // checkbox: same size, normal write, mtime untouched by anyone.
    std::thread::sleep(Duration::from_millis(300));
    fs::write(d.path().join("Tasks.md"), "- [x] buy milk\n- [ ] call bob\nnotes: ").unwrap();
    // The watcher fires; the scanner sees nothing.
    let seen = v.rescan_paths(&["Tasks.md".to_string()]).unwrap();
    // The user keeps typing; the next autosave still uses Cairn's base hash.
    let w = v.write_note("Tasks.md", "- [ ] buy milk\n- [ ] call bob\nnotes: hi", Some(&saved.hash));
    let disk = fs::read_to_string(d.path().join("Tasks.md")).unwrap();
    println!("rescan saw: {seen:?}\nwrite: {:?}\ndisk: {disk:?}", w.as_ref().map(|_| "Ok"));
    assert!(
        matches!(w, Err(CoreError::Conflict(_))) || disk.contains("[x]"),
        "checkbox ticked by another program was silently reverted; rescan saw {seen:?}"
    );
}

fn natural_race(root: &Path, gap: Duration, rounds: u32) -> (u32, u32) {
    let pad = "x".repeat(200);
    fs::write(root.join("n.md"), format!("a{pad}")).unwrap();
    let v = Vault::open(Arc::new(StdFs::new(root, TrashMode::Vault).unwrap())).unwrap();
    let mut base = v.read_note("n.md").unwrap().hash;
    let (mut lost, mut caught) = (0, 0);
    for i in 0..rounds {
        let mine = format!("C{:06}{pad}", i);
        let r = v.write_note("n.md", &mine, Some(&base)).unwrap();
        if !gap.is_zero() {
            std::thread::sleep(gap);
        }
        // External same-size in-place write, no mtime manipulation.
        fs::write(root.join("n.md"), format!("E{:06}{pad}", i)).unwrap();
        let _ = v.rescan_paths(&["n.md".to_string()]).unwrap();
        match v.write_note("n.md", &format!("D{:06}{pad}", i), Some(&r.hash)) {
            Ok(w) => {
                lost += 1;
                base = w.hash;
            }
            Err(CoreError::Conflict(_)) => {
                caught += 1;
                base = v.read_note("n.md").unwrap().hash;
            }
            Err(e) => panic!("{e:?}"),
        }
    }
    (lost, caught)
}

#[test]
fn dl07_fine_grained_fs_natural_race_rate() {
    let tmpfs = tempfile::tempdir().unwrap();
    let disk = tempfile::Builder::new().prefix("adv-verify-dl07-").tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    for (name, root) in [("tmpfs", tmpfs.path()), ("disk", disk.path())] {
        for gap in [Duration::ZERO, Duration::from_millis(2)] {
            let (lost, caught) = natural_race(root, gap, 300);
            println!("{name}: external write {gap:?} after Cairn's save: {lost} silently overwritten, {caught} conflicts (of 300)");
            assert_eq!(lost, 0, "{name}, {gap:?}: external writes silently overwritten");
        }
    }
}
