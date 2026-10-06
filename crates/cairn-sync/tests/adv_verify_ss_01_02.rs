//! Further regression tests for FINDING-004.
//!
//! The simulated `CaseInsensitiveFs` and the Linux casefold tmpfs (see
//! adv_verify_ss_01.rs) both keep the OLD name when a file is renamed over a
//! differently-cased existing entry (`rename(tmp, "ideas.md")` leaves the
//! file called `Ideas.md`). APFS/HFS+ and NTFS are expected to take the NEW
//! name given to rename(2)/MoveFileEx instead. `StdFs::write` is a temp file
//! plus rename, so on macOS/Windows the overwritten file most likely ends up
//! called `ideas.md`. This file models that variant, to check that no note
//! is lost whichever name survives:
//!
//! * the two-notes case: both notes stay in a live vault. (With the defect,
//!   the mac pushed a DELETE of `Ideas.md`, so the Linux device moved UPPER
//!   to its trash, and the mac's own copy was destroyed outright.)
//! * the unsynced-note case: the mac's never-uploaded note survives in a
//!   vault, a trash or the server (with the defect it was lost for good).
//!
//! Run: cargo test -p cairn-sync --test adv_verify_ss_01_02 -- --nocapture --test-threads=1

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cairn_core::{CoreError, FileStat, StdFs, TrashMode, VaultFs};
use common::*;

/// Case-insensitive, case-preserving; a write whose name differs only in case
/// from an existing file REPLACES that file and the entry takes the NEW name
/// (what rename-over-existing does on APFS/NTFS, as far as we know).
struct NewNameCiFs {
    inner: StdFs,
    root: PathBuf,
}

impl NewNameCiFs {
    // Passed to Device::with_fs, which takes the file system as a trait object.
    #[allow(clippy::new_ret_no_self)]
    fn new(root: &Path) -> Arc<dyn VaultFs> {
        Arc::new(NewNameCiFs { inner: StdFs::new(root, TrashMode::Vault).unwrap(), root: root.to_path_buf() })
    }

    fn resolve(&self, p: &str) -> String {
        let mut real: Vec<String> = Vec::new();
        for comp in p.split('/').filter(|c| !c.is_empty()) {
            let mut dir = self.root.clone();
            for r in &real {
                dir.push(r);
            }
            let found = fs::read_dir(&dir).ok().and_then(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .find(|n| n.to_lowercase() == comp.to_lowercase())
            });
            real.push(found.unwrap_or_else(|| comp.to_string()));
        }
        real.join("/")
    }
}

impl VaultFs for NewNameCiFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        self.inner.list(&self.resolve(dir))
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        Ok(self.inner.stat(&self.resolve(path))?.map(|mut s| {
            s.path = path.to_string();
            s
        }))
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.inner.read(&self.resolve(path))
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        let parent = self.resolve(cairn_core::path::parent(path));
        let target = cairn_core::path::join(&parent, cairn_core::path::file_name(path));
        let existing = self.resolve(path);
        if existing != target && self.root.join(&existing).is_file() {
            // rename(tmp, target) replaces the differently-cased entry: its
            // inode is gone (no trash), and the new entry carries `target`.
            fs::remove_file(self.root.join(&existing)).unwrap();
        }
        let mut st = self.inner.write(&target, data)?;
        st.path = path.to_string();
        Ok(st)
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.create_dir(&self.resolve(path))
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        let rf = self.resolve(from);
        let rt = self.resolve(to);
        if rf.to_lowercase() != rt.to_lowercase() && self.inner.stat(&rt)?.is_some() {
            return Err(CoreError::AlreadyExists(to.to_string()));
        }
        let parent = cairn_core::path::parent(&rt);
        let target = cairn_core::path::join(parent, cairn_core::path::file_name(to));
        self.inner.rename(&rf, &target)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.remove(&self.resolve(path))
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.inner.remove_empty_dir(&self.resolve(path))
    }
    fn describe(&self) -> String {
        format!("case-insensitive (new name wins) {}", self.inner.describe())
    }
}

fn server_text(d: &Device) -> String {
    let mut out = String::new();
    for seq in 1..200 {
        match d.engine.revision_content(seq) {
            Ok(p) => out.push_str(&format!("[{seq} {}] {}", p.path, String::from_utf8_lossy(&p.data))),
            Err(_) => break,
        }
    }
    out
}

#[test]
fn two_notes_differing_only_in_case_new_name_wins() {
    let srv = server();
    let mut linux = Device::new(
        &srv,
        "linux",
        &[("Ideas.md", "UPPER: the only copy of these ideas, a longer note\n"), ("ideas.md", "lower: a different note\n")],
    );
    linux.sync();
    let mut mac = Device::with_fs(&srv, "mac", &[], NewNameCiFs::new);
    let r = mac.try_sync();
    eprintln!("mac first sync: {:?}", r.as_ref().map(|r| (&r.conflicts, &r.changes)).map_err(|e| e.to_string()));
    for _ in 0..2 {
        let _ = linux.try_sync();
        let _ = mac.try_sync();
    }
    let srv_text = server_text(&linux);
    eprintln!(
        "linux files: {:?}\nlinux trash: {:?}\nmac files: {:?}\nmac trash: {:?}\nserver: {srv_text}",
        linux.files(),
        linux.trash_text(),
        mac.files(),
        mac.trash_text()
    );
    let live = format!("{:?}{:?}", linux.files(), mac.files());
    eprintln!(
        "UPPER live: {}, in linux trash: {}, in mac trash: {}, in server history: {}",
        live.contains("UPPER"),
        linux.trash_text().contains("UPPER"),
        mac.trash_text().contains("UPPER"),
        srv_text.contains("UPPER")
    );
    assert!(live.contains("UPPER") && live.contains("lower: a different note"), "a note vanished from every live vault");
}

#[test]
fn unsynced_local_note_new_name_wins() {
    let srv = server();
    let mut linux = Device::new(&srv, "linux", &[("note.md", "from linux\n")]);
    linux.sync();
    let mut mac = Device::with_fs(&srv, "mac", &[("Note.md", "only on the mac, never synced\n")], NewNameCiFs::new);
    let r = mac.try_sync();
    let _ = linux.try_sync();
    let _ = mac.try_sync();
    let srv_text = server_text(&linux);
    eprintln!(
        "mac sync: {:?}\nmac files {:?}\nlinux files {:?}\nserver: {srv_text}",
        r.map(|r| r.conflicts).map_err(|e| e.to_string()),
        mac.files(),
        linux.files()
    );
    let everywhere = format!("{}{}{}", mac.all_text(), linux.all_text(), srv_text);
    assert!(everywhere.contains("only on the mac, never synced"), "the mac's own note is gone from both vaults, both trashes and the server");
}
