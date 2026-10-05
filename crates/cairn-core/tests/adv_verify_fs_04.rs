//! Reproduction for FINDING-132: a folder replaced by a file must not abort
//! a scan with ENOTDIR.
//!
//!   cargo test -p cairn-core --test adv_verify_fs_04 -- --include-ignored --nocapture
//!
//! Two code paths are involved:
//! - `StdFs::list_into` (fs.rs, the `read_dir` match): race between
//!   `fs::metadata` saying "folder" and `read_dir` on it. Only hit when the
//!   swap lands in that tiny window during a full scan.
//! - `StdFs::stat_abs` (fs.rs, the `fs::metadata` match): no race needed. A
//!   watcher hint for `toggle/a.md` that is processed after `toggle` became a
//!   file must give `None`, not ENOTDIR, or `rescan_paths` would fail for the
//!   whole batch. notify-debouncer-full expires events one by one, so such a
//!   batch can be delivered on its own.

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cairn_core::{StdFs, TrashMode, Vault, VaultFs};

fn vault_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = Vault::open(Arc::new(StdFs::new(d.path(), TrashMode::Vault).unwrap())).unwrap();
    (d, v)
}

/// Deterministic, no race: a stale hint below a folder that is now a file.
#[test]
fn fs04_stale_hint_below_folder_that_became_file() {
    let (d, v) = vault_with(&[("toggle/a.md", "a"), ("other.md", "o")]);
    fs::remove_file(d.path().join("toggle/a.md")).unwrap();
    fs::remove_dir(d.path().join("toggle")).unwrap();
    fs::write(d.path().join("toggle"), "now a file").unwrap();
    fs::write(d.path().join("new.md"), "new").unwrap();

    let fsx = StdFs::new(d.path(), TrashMode::Vault).unwrap();
    println!("stat(toggle/a.md) = {:?}", fsx.stat("toggle/a.md"));

    let batch = v.rescan_paths(&["new.md".into(), "toggle/a.md".into()]);
    println!("batch = {batch:?}");
    assert!(batch.is_ok(), "{batch:?}");
    assert!(v.index().note("new.md").is_some());
}

/// Control: the later batch that mentions the parent heals the index, so
/// even a failed stale hint would be transient in the git-checkout case.
#[test]
fn fs04_later_parent_batch_heals_index() {
    let (d, v) = vault_with(&[("toggle/a.md", "a")]);
    fs::remove_file(d.path().join("toggle/a.md")).unwrap();
    fs::remove_dir(d.path().join("toggle")).unwrap();
    fs::write(d.path().join("toggle"), "now a file").unwrap();
    let first = v.rescan_paths(&["toggle/a.md".into()]);
    println!("first batch = {first:?}");
    let second = v.rescan_paths(&["toggle".into()]).expect("parent batch");
    println!("second batch = {second:?}");
    let idx = v.index();
    assert!(idx.entry("toggle/a.md").is_none());
    assert_eq!(idx.entry("toggle").map(|e| e.kind), Some(cairn_core::EntryKind::File));
}

/// Measures the race at a slower, more realistic swap rate (one swap every
/// ~20 ms) against back-to-back full listings. Informational only.
#[test]
#[ignore = "slow: informational race-rate probe (about 3 s)"]
fn fs04_race_rate_at_slow_swap() {
    let (d, _v) = vault_with(&[("a/one.md", "1"), ("b/two.md", "2")]);
    let stop = Arc::new(AtomicBool::new(false));
    let swaps = Arc::new(AtomicUsize::new(0));
    let (s2, sw2, root) = (stop.clone(), swaps.clone(), d.path().to_path_buf());
    let churn = std::thread::spawn(move || {
        while !s2.load(Ordering::Relaxed) {
            let x = root.join("toggle");
            let _ = fs::create_dir(&x);
            let _ = fs::write(x.join("a.md"), "a");
            std::thread::sleep(Duration::from_millis(10));
            let _ = fs::remove_file(x.join("a.md"));
            let _ = fs::remove_dir(&x);
            let _ = fs::write(&x, "now a file");
            std::thread::sleep(Duration::from_millis(10));
            let _ = fs::remove_file(&x);
            sw2.fetch_add(1, Ordering::Relaxed);
        }
    });
    let fsx = StdFs::new(d.path(), TrashMode::Vault).unwrap();
    let (mut ok, mut err) = (0usize, 0usize);
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) {
        match fsx.list("") {
            Ok(_) => ok += 1,
            Err(_) => err += 1,
        }
    }
    stop.store(true, Ordering::Relaxed);
    churn.join().unwrap();
    println!("swaps={} lists ok={ok} err={err}", swaps.load(Ordering::Relaxed));
    assert_eq!(err, 0);
}
