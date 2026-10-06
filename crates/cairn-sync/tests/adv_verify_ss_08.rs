//! Regression tests for FINDING-147 (the device name was copied into
//! conflict copy names unsanitized, and a name the receiving file system
//! refused stopped sync).
//!
//! * `conflict_copy_name_obeys_cairns_own_name_rules`: an engine that builds
//!   conflict names from the device name with only `/ \ :` replaced makes
//!   Cairn itself create names that its own
//!   `cairn_core::path::validate_name` / `FORBIDDEN_NAME_CHARS` refuse.
//! * `saf_like_receiver_does_not_stall`: models how Android's built-in
//!   storage provider (com.android.internal.content.FileSystemProvider,
//!   used by ExternalStorageProvider for shared storage and SD cards)
//!   handles such names. Its createDocument/renameDocument call
//!   FileUtils.buildValidFatFilename, which REPLACES `"*/:<>?\|` and
//!   control characters with `_` instead of refusing them (checked by
//!   disassembling framework.jar from the Android 15 emulator). Cairn's
//!   SafPlugin.write then reports the original path. This test shows what
//!   that does when a device named "Sam's Pixel?" makes a conflict copy.
//!
//! Run: cargo test -p cairn-sync --test adv_verify_ss_08 -- --nocapture --test-threads=1

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::path::Path;
use std::sync::Arc;

use cairn_core::{CoreError, FileStat, StdFs, TrashMode, VaultFs};
use common::*;

/// A `VaultFs` that behaves like Cairn's SafFs on top of Android's
/// FileSystemProvider: new names are FAT-sanitized (`_`) and made unique
/// (" (1)"), lookups are by exact name, and `write` returns the path it was
/// asked for (SafPlugin.kt `entry(a.path, d)`).
struct SafLikeFs {
    inner: StdFs,
}

impl SafLikeFs {
    // Passed to Device::with_fs, which takes the file system as a trait object.
    #[allow(clippy::new_ret_no_self)]
    fn new(root: &Path) -> Arc<dyn VaultFs> {
        Arc::new(SafLikeFs { inner: StdFs::new(root, TrashMode::Vault).unwrap() })
    }

    fn sanitize(name: &str) -> String {
        name.chars().map(|c| if c < ' ' || c == '\u{7f}' || "\"*/:<>?\\|".contains(c) { '_' } else { c }).collect()
    }

    fn unique(&self, parent: &str, name: &str) -> String {
        let join = |n: &str| if parent.is_empty() { n.to_string() } else { format!("{parent}/{n}") };
        let first = join(name);
        if self.inner.stat(&first).ok().flatten().is_none() {
            return first;
        }
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        for n in 1.. {
            let p = join(&format!("{stem} ({n}){ext}"));
            if self.inner.stat(&p).ok().flatten().is_none() {
                return p;
            }
        }
        unreachable!()
    }
}

impl VaultFs for SafLikeFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        self.inner.list(dir)
    }
    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        self.inner.stat(path)
    }
    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        self.inner.read(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        if self.inner.stat(path)?.is_some() {
            return self.inner.write(path, data);
        }
        let parent = cairn_core::path::parent(path);
        let actual = self.unique(parent, &Self::sanitize(cairn_core::path::file_name(path)));
        let mut st = self.inner.write(&actual, data)?;
        st.path = path.to_string();
        Ok(st)
    }
    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        if self.inner.stat(from)?.is_none() {
            return Err(CoreError::NotFound(from.to_string()));
        }
        if !from.eq_ignore_ascii_case(to) && self.inner.stat(to)?.is_some() {
            return Err(CoreError::AlreadyExists(to.to_string()));
        }
        let name = cairn_core::path::file_name(to);
        let target = if cairn_core::path::file_name(from) == name {
            to.to_string()
        } else {
            self.unique(cairn_core::path::parent(to), &Self::sanitize(name))
        };
        self.inner.rename(from, &target)
    }
    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        self.inner.remove(path)
    }
    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        self.inner.remove_empty_dir(path)
    }
    fn describe(&self) -> String {
        format!("saf-like {}", self.inner.describe())
    }
}

/// Cairn refuses `? " * < > | [ ] # ^` in names (path.rs FORBIDDEN_NAME_CHARS),
/// so its own conflict copies must not carry the raw device name.
#[test]
fn conflict_copy_name_obeys_cairns_own_name_rules() {
    let mut bad = Vec::new();
    for dev in ["Sam's Pixel?", "Phone \"work\"", "Tab|2", "Phone #2", "Pixel [old]"] {
        let srv = server();
        let mut a = Device::new(&srv, dev, &[("n.md", "base\n")]);
        a.sync();
        let mut b = Device::new(&srv, "laptop", &[]);
        b.sync();
        b.write("n.md", "laptop\n");
        a.write("n.md", "pixel\n");
        b.sync();
        let r = a.sync();
        assert_eq!(r.conflicts.len(), 1, "{dev}: {:?}", r.conflicts);
        let name = cairn_core::path::file_name(&r.conflicts[0]).to_string();
        if let Err(e) = cairn_core::path::validate_name(&name) {
            bad.push(format!("device {dev:?} -> {name:?}: {e}"));
        }
    }
    assert!(bad.is_empty(), "conflict copies that Cairn's own validate_name refuses:\n{}", bad.join("\n"));
}

/// A device named "Sam's Pixel?" makes a conflict copy, and a receiving
/// device is on an SAF-like file system (Android shared storage / SD card
/// through the system picker).
/// Prints what happens; asserts only that sync keeps working there.
#[test]
fn saf_like_receiver_does_not_stall() {
    let srv = server();
    let mut a = Device::new(&srv, "Sam's Pixel?", &[("n.md", "base\n")]);
    a.sync();
    let mut w = Device::with_fs(&srv, "android-saf", &[], SafLikeFs::new);
    w.sync();
    let mut b = Device::new(&srv, "laptop", &[]);
    b.sync();
    b.write("n.md", "laptop\n");
    a.write("n.md", "pixel\n");
    b.sync();
    let r = a.sync();
    eprintln!("conflict copy: {:?}", r.conflicts);
    let rw = w.try_sync();
    eprintln!("saf sync 1: {:?}", rw.as_ref().map(|r| (r.pulled, r.conflicts.clone())).map_err(|e| e.to_string()));
    a.write("later.md", "later\n");
    a.sync();
    let rw2 = w.try_sync();
    eprintln!("saf sync 2: {:?}", rw2.as_ref().map(|r| (r.pulled, r.conflicts.clone())).map_err(|e| e.to_string()));
    let conv = try_converge(&mut [&mut a, &mut w, &mut b]);
    eprintln!("converge: {conv:?}");
    eprintln!("pixel: {:?}", a.paths());
    eprintln!("saf:   {:?}", w.paths());
    eprintln!("laptop:{:?}", b.paths());
    assert!(rw.is_ok() && rw2.is_ok(), "saf device: {:?} / {:?}", rw.err().map(|e| e.to_string()), rw2.err().map(|e| e.to_string()));
    assert!(w.exists("later.md"));
}

/// Same, but the device that creates the conflict copy is itself on the
/// SAF-like file system (a phone named "Sam's Pixel?" with a shared folder).
#[test]
fn saf_like_device_creating_the_conflict_copy() {
    let srv = server();
    let mut a = Device::with_fs(&srv, "Sam's Pixel?", &[("n.md", "base\n")], SafLikeFs::new);
    a.sync();
    let mut b = Device::new(&srv, "laptop", &[]);
    b.sync();
    b.write("n.md", "laptop\n");
    a.write("n.md", "pixel\n");
    b.sync();
    let r = a.try_sync();
    eprintln!("pixel sync: {:?}", r.as_ref().map(|r| r.conflicts.clone()).map_err(|e| e.to_string()));
    let conv = try_converge(&mut [&mut a, &mut b]);
    eprintln!("converge: {conv:?}");
    eprintln!("pixel: {:?}", a.files());
    eprintln!("laptop:{:?}", b.files());
    assert!(r.is_ok());
}
