//! Timing-free regression test for FINDING-129.
//!
//! The desktop watcher (`start` in app/src-tauri/src/watcher.rs) hands each
//! notify-debouncer-full batch to `Vault::rescan_paths` on its own. The
//! debouncer keys queues by path and, on a folder rename, moves only the
//! folder's own queue (notify-debouncer-full 0.7 `push_rename_event`), so an
//! event queued a moment earlier for a child (`IN_OPEN` from any reader, or
//! Cairn's own save) is flushed in an earlier batch under the OLD child path.
//!
//! This test replays exactly those two batches against the core:
//!   batch 1: ["dir/C.md"]       (child event, flushed first)
//!   batch 2: ["dir", "dir2"]    (the rename's from/to paths)
//! The control replays only batch 2 and gets a clean `Renamed`.
//!
//! If `Vault::rescan_paths` took a hint that no longer exists as a deletion
//! of just that path, batch 1 would remove dir/C.md from the index; batch 2
//! would then compare the folder signature of `dir` (empty by then) with
//! `dir2` ([("C.md", 8)]), find no match, and report Deleted dir + Created
//! dir2 + Created dir2/C.md. A stale hint whose folder is gone too is
//! rescanned from the nearest folder that still exists instead.
//!
//! Run:
//!   cargo test -p cairn-core --test adv_verify_dl_09 -- --nocapture

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cairn_core::{Change, StdFs, TrashMode, Vault};

fn open(p: &Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap())).unwrap()
}

fn setup() -> (tempfile::TempDir, Vault) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("dir")).unwrap();
    fs::write(t.path().join("dir/C.md"), "charlie\n").unwrap();
    fs::write(t.path().join("Z.md"), "z\n").unwrap();
    let v = open(t.path());
    (t, v)
}

fn describe(changes: &[Change]) -> Vec<String> {
    changes
        .iter()
        .map(|c| match c {
            Change::Created { entry } => format!("Created {}", entry.path),
            Change::Modified { entry } => format!("Modified {}", entry.path),
            Change::Deleted { path, .. } => format!("Deleted {path}"),
            Change::Renamed { from, entry } => format!("Renamed {from} -> {}", entry.path),
        })
        .collect()
}

#[test]
fn control_folder_rename_in_one_batch_is_a_rename() {
    let (t, v) = setup();
    fs::rename(t.path().join("dir"), t.path().join("dir2")).unwrap();
    let ch = describe(&v.rescan_paths(&["dir".into(), "dir2".into()]).unwrap());
    eprintln!("control: {ch:?}");
    assert_eq!(ch, vec!["Renamed dir -> dir2".to_string()]);
}

#[test]
fn folder_rename_after_child_event_in_previous_batch_is_still_a_rename() {
    let (t, v) = setup();
    fs::rename(t.path().join("dir"), t.path().join("dir2")).unwrap();
    // Batch 1: the child's IN_OPEN / IN_CLOSE_WRITE, flushed ~one tick before
    // the rename's events (paths are old, the folder is already gone).
    let b1 = describe(&v.rescan_paths(&["dir/C.md".into()]).unwrap());
    // Batch 2: the rename itself.
    let b2 = describe(&v.rescan_paths(&["dir".into(), "dir2".into()]).unwrap());
    eprintln!("batch 1: {b1:?}");
    eprintln!("batch 2: {b2:?}");
    // The index ends up right either way; what matters is what the UI is
    // told: a Deleted would close clean tabs in dir/ and give dirty ones the
    // "deleted or moved" banner instead of following the Renamed.
    let paths: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    eprintln!("index after: {paths:?}");
    let all: Vec<String> = b1.into_iter().chain(b2).collect();
    assert!(
        !all.iter().any(|c| c.starts_with("Deleted")),
        "folder rename reported as deletions: {all:?}"
    );
    assert!(all.contains(&"Renamed dir -> dir2".to_string()), "no rename reported: {all:?}");
}
