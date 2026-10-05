//! Reproduction for FINDING-134 (folder-rename detection in
//! `Vault::apply_diff` must not be cubic).
//!
//! fs12 in adv_fs_scan.rs varies the number of folders and the number of
//! files together, so it cannot tell the folder-rename loop (deleted dirs x
//! created dirs x created entries) from the file-level passes (file-rename
//! search and the top-most-deletion filter). These tests separate the two:
//! one keeps the number of notes fixed and changes only the number of
//! folders, the other keeps a single folder and changes the number of notes.
//!
//! Run with:
//!   cargo test -p cairn-core --test adv_verify_fs_12 -- --ignored --nocapture --test-threads=1

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use cairn_core::{Change, StdFs, TrashMode, Vault};

fn open(p: &Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap())).unwrap()
}

/// `folders` folders of `per` notes are deleted and `folders` different
/// folders of `per` different notes are created; returns rescan seconds.
fn replace(folders: usize, per: usize) -> f64 {
    let d = tempfile::tempdir().unwrap();
    for i in 0..folders {
        fs::create_dir_all(d.path().join(format!("f{i:04}"))).unwrap();
        for j in 0..per {
            fs::write(d.path().join(format!("f{i:04}/n{j}.md")), format!("{i} {j}")).unwrap();
        }
    }
    let v = open(d.path());
    for i in 0..folders {
        fs::remove_dir_all(d.path().join(format!("f{i:04}"))).unwrap();
        fs::create_dir_all(d.path().join(format!("g{i:04}"))).unwrap();
        for j in 0..per {
            fs::write(d.path().join(format!("g{i:04}/m{j}.md")), format!("other {i} {j}")).unwrap();
        }
    }
    let t = Instant::now();
    let c = v.rescan().unwrap();
    let secs = t.elapsed().as_secs_f64();
    let del = c.iter().filter(|c| matches!(c, Change::Deleted { .. })).count();
    assert_eq!(del, folders, "{c:?}");
    secs
}

#[test]
fn fs12_folder_loop_same_notes_more_folders_is_much_slower() {
    // 500 deleted notes and 500 created notes in both runs; only the number
    // of folders they sit in changes (1 vs 500).
    let one = replace(1, 500);
    let many = replace(500, 1);
    let ratio = many / one;
    println!("500 notes in 1 folder: {one:.3} s; in 500 folders: {many:.3} s; ratio {ratio:.1}");
    assert!(ratio < 3.0, "same number of notes in 500 folders took {ratio:.1}x the time ({one:.3} s -> {many:.3} s)");
}

#[test]
fn fs12_file_level_rename_search_is_quadratic() {
    // One folder of n notes deleted, one folder of n different notes created:
    // the folder-rename loop runs once, so only the file-level passes scale.
    let small = replace(1, 500);
    let big = replace(1, 2000);
    let ratio = big / small;
    println!("1 folder of 500 notes: {small:.3} s; of 2000 notes: {big:.3} s; ratio {ratio:.1} (linear would be about 4)");
    assert!(ratio < 8.0, "4x the notes took {ratio:.1}x the time ({small:.3} s -> {big:.3} s)");
}

/// Control: a real reorganization (folders renamed, contents unchanged) with
/// the same counts. The signatures match early, so this stays fast.
#[test]
fn fs12_control_real_folder_renames_are_fast() {
    let d = tempfile::tempdir().unwrap();
    let n = 200;
    for i in 0..n {
        fs::create_dir_all(d.path().join(format!("f{i:04}"))).unwrap();
        for j in 0..10 {
            fs::write(d.path().join(format!("f{i:04}/n{j}.md")), format!("{i} {j}")).unwrap();
        }
    }
    let v = open(d.path());
    // Reverse the order so the matching candidate is never the first one.
    for i in 0..n {
        fs::rename(d.path().join(format!("f{i:04}")), d.path().join(format!("g{:04}", n - 1 - i))).unwrap();
    }
    let t = Instant::now();
    let c = v.rescan().unwrap();
    let secs = t.elapsed().as_secs_f64();
    let renamed = c.iter().filter(|c| matches!(c, Change::Renamed { .. })).count();
    println!("200 folders renamed (reverse order): {secs:.3} s, {renamed} renames");
    assert_eq!(renamed, n);
}

/// Probe: rescan time for `FS12_LAYOUTS="folders x per, ..."` (default below).
/// Separates the file-level quadratic passes (1 folder of many notes) from
/// the folder-rename loop (many folders of 1 note).
#[test]
#[ignore = "slow: scaling probe (prints timings, no failure expected)"]
fn fs12_probe_layouts() {
    let spec = std::env::var("FS12_LAYOUTS")
        .unwrap_or_else(|_| "1x1000,1x2000,1x4000,1000x1,2000x1,100x10,200x10,400x10".into());
    for l in spec.split(',') {
        let (f, p) = l.trim().split_once('x').unwrap();
        let (f, p): (usize, usize) = (f.parse().unwrap(), p.parse().unwrap());
        let s = replace(f, p);
        println!("{f:>5} folders x {p:>4} notes ({:>6} notes): {s:.3} s", f * p);
    }
}
