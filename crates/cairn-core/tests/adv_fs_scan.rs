//! Adversarial tests: the scanner and change detection at the
//! edges: unreadable folders, races, symlink loops, bulk changes, rename
//! detection, very large notes.
//!
//! Tests named `fsNN_*` come from findings; those not ignored are regression
//! tests for fixed ones. Run one with:
//!   cargo test -p cairn-core --test adv_fs_scan -- --exact <name>
//! Ignored tests are slow coverage (a "slow:" reason) or document behaviour
//! that is not a defect (by design); run one of those with:
//!   cargo test -p cairn-core --test adv_fs_scan -- --ignored --exact <name>
//! Run the rest (about 10 s in a debug build) with:
//!   cargo test -p cairn-core --test adv_fs_scan

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cairn_core::{Change, CoreError, StdFs, TrashMode, Vault, VaultFs};

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = open(d.path());
    (d, v)
}

fn try_open(p: &Path) -> cairn_core::Result<Vault> {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap()))
}

fn open(p: &Path) -> Vault {
    try_open(p).unwrap()
}

fn chmod(p: &Path, mode: u32) {
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

/// Everything the index knows: paths with note contents.
fn snapshot(v: &Vault) -> Vec<(String, Option<String>)> {
    let idx = v.index();
    let mut out: Vec<(String, Option<String>)> =
        idx.entries().map(|e| (e.path.clone(), idx.note(&e.path).map(|n| n.content.clone()))).collect();
    out.sort();
    out
}

fn count<F: Fn(&Change) -> bool>(c: &[Change], f: F) -> usize {
    c.iter().filter(|c| f(c)).count()
}

fn is_renamed(c: &Change) -> bool {
    matches!(c, Change::Renamed { .. })
}

// ---------------------------------------------------------------------------
// FINDING-049: one unreadable subfolder does not fail the whole scan
// ---------------------------------------------------------------------------

#[test]
fn fs03_unreadable_subfolder_does_not_fail_open() {
    // A vault at the root of an ext4 USB stick has a root-owned 0700 lost+found.
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("ok.md"), "fine").unwrap();
    fs::create_dir(d.path().join("lost+found")).unwrap();
    chmod(&d.path().join("lost+found"), 0o000);
    let r = try_open(d.path());
    chmod(&d.path().join("lost+found"), 0o755);
    assert!(r.is_ok(), "Vault::open failed: {:?}", r.err());
    assert!(r.unwrap().index().note("ok.md").is_some());
}

#[test]
fn fs03_unreadable_subfolder_does_not_fail_rescan() {
    let (d, v) = setup(&[("ok.md", "fine"), ("private/x.md", "x")]);
    chmod(&d.path().join("private"), 0o000);
    fs::write(d.path().join("new.md"), "made in another editor").unwrap();
    let full = v.rescan();
    let hinted = v.rescan_paths(&["new.md".into(), "private".into()]);
    chmod(&d.path().join("private"), 0o755);
    assert!(full.is_ok(), "rescan: {:?}", full.err());
    assert!(hinted.is_ok(), "rescan_paths: {:?}", hinted.err());
}

#[test]
fn unreadable_file_does_not_break_open_or_rescan() {
    let (d, v0) = setup(&[("ok.md", "fine"), ("secret.md", "s")]);
    drop(v0);
    chmod(&d.path().join("secret.md"), 0o000);
    let v = try_open(d.path());
    let r = v.as_ref().map(|v| v.rescan().is_ok());
    chmod(&d.path().join("secret.md"), 0o644);
    assert!(r.as_ref().is_ok_and(|ok| *ok), "open or rescan failed");
    let v = v.unwrap();
    assert!(v.index().entry("secret.md").is_some());
    assert!(v.index().note("ok.md").is_some());
}

// ---------------------------------------------------------------------------
// FINDING-132: a folder replaced by a file mid-scan does not abort the scan
// ---------------------------------------------------------------------------

#[test]
fn fs04_scan_survives_folder_replaced_by_file() {
    let (d, v) = setup(&[("a/one.md", "1"), ("b/two.md", "2")]);
    let stop = Arc::new(AtomicBool::new(false));
    let (s2, root) = (stop.clone(), d.path().to_path_buf());
    // Like `git checkout` of a branch where "toggle" is a file instead of a folder.
    let churn = std::thread::spawn(move || {
        while !s2.load(Ordering::Relaxed) {
            let x = root.join("toggle");
            let _ = fs::create_dir(&x);
            let _ = fs::write(x.join("a.md"), "a");
            let _ = fs::remove_file(x.join("a.md"));
            let _ = fs::remove_dir(&x);
            let _ = fs::write(&x, "now a file");
            let _ = fs::remove_file(&x);
        }
    });
    let fsx = StdFs::new(d.path(), TrashMode::Vault).unwrap();
    let mut errors = Vec::new();
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) && errors.len() < 5 {
        if let Err(e) = fsx.list("") {
            errors.push(format!("list: {e}"));
        }
        if let Err(e) = v.rescan() {
            errors.push(format!("rescan: {e}"));
        }
    }
    stop.store(true, Ordering::Relaxed);
    churn.join().unwrap();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn scan_survives_files_appearing_and_vanishing() {
    let mut files = Vec::new();
    for i in 0..200 {
        files.push((format!("d{}/n{i}.md", i % 10), format!("note {i}")));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (d, v) = setup(&refs);
    let stop = Arc::new(AtomicBool::new(false));
    let (s2, root) = (stop.clone(), d.path().to_path_buf());
    let churn = std::thread::spawn(move || {
        let mut i = 0u64;
        while !s2.load(Ordering::Relaxed) {
            let dir = root.join(format!("d{}", i % 10));
            let f = dir.join(format!("churn{}.md", i % 37));
            if i % 2 == 0 {
                let _ = fs::write(&f, format!("c{i}"));
            } else {
                let _ = fs::remove_file(&f);
            }
            // whole folders appearing and disappearing
            let tmpdir = root.join(format!("burst{}", i % 3));
            if i % 5 == 0 {
                let _ = fs::create_dir(&tmpdir);
                let _ = fs::write(tmpdir.join("x.md"), "x");
            } else if i % 5 == 3 {
                let _ = fs::remove_dir_all(&tmpdir);
            }
            i += 1;
        }
    });
    let mut errors = Vec::new();
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(2) {
        if let Err(e) = try_open(d.path()) {
            errors.push(format!("open: {e}"));
        }
        if let Err(e) = v.rescan() {
            errors.push(format!("rescan: {e}"));
        }
        if let Err(e) = v.rescan_paths(&["d1".into(), "burst0".into(), "d2/churn3.md".into()]) {
            errors.push(format!("rescan_paths: {e}"));
        }
    }
    stop.store(true, Ordering::Relaxed);
    churn.join().unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    v.rescan().unwrap();
    assert_eq!(snapshot(&v), snapshot(&open(d.path())), "index differs from a fresh open after the churn");
}

// ---------------------------------------------------------------------------
// FINDING-012: symlink loops
// ---------------------------------------------------------------------------

#[test]
fn fs05_single_self_loop_adds_no_duplicates() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("n.md"), "unique words").unwrap();
    symlink(".", d.path().join("loop")).unwrap();
    let v = open(d.path());
    let notes = v.index().note_count();
    let hits = v.search("unique", 1000).len();
    assert_eq!((notes, hits), (1, 1), "one note became {notes} notes and {hits} search hits");
}

#[test]
fn a_symlink_loop_seen_by_the_watcher_adds_no_duplicates() {
    let (d, v) = setup(&[("n.md", "unique words"), ("a/m.md", "more words")]);
    symlink(".", d.path().join("loop")).unwrap();
    symlink("..", d.path().join("a/up")).unwrap();
    // The watcher reports the new links, and paths below them.
    v.rescan_paths(&["loop".into(), "a/up".into()]).unwrap();
    v.rescan_paths(&["loop/n.md".into(), "a/up/n.md".into(), "loop/a".into()]).unwrap();
    let mut paths: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    paths.sort();
    assert_eq!(paths, vec!["a", "a/m.md", "n.md"]);
    // The watcher can report a change in `a` (or the vault) under a loop's
    // name, as it watches each folder once: it is a change to the folder
    // the loop leads to.
    fs::write(d.path().join("a/new.md"), "new words").unwrap();
    fs::remove_file(d.path().join("a/m.md")).unwrap();
    fs::write(d.path().join("top.md"), "top words").unwrap();
    v.rescan_paths(&["loop/a/new.md".into(), "a/up/a/m.md".into(), "loop/x.md".into(), "a/up/top.md".into()]).unwrap();
    let mut paths: Vec<String> = v.entries().into_iter().map(|e| e.path).collect();
    paths.sort();
    assert_eq!(paths, vec!["a", "a/new.md", "n.md", "top.md"]);
}

#[test]
fn fs05_two_symlink_loops_do_not_hang_open() {
    if let Ok(dir) = std::env::var("ADV_FS_LOOP_CHILD") {
        // Child process: try to open the vault and report.
        let v = open(Path::new(&dir));
        println!("child opened {} entries", v.entries().len());
        return;
    }
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("n.md"), "x").unwrap();
    symlink(".", d.path().join("loop1")).unwrap();
    symlink(".", d.path().join("loop2")).unwrap();
    // Run the open in a child process so the runaway listing can be killed.
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fs05_two_symlink_loops_do_not_hang_open", "--nocapture", "--test-threads=1"])
        .env("ADV_FS_LOOP_CHILD", d.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let t = Instant::now();
    let finished = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break Some(st);
        }
        if t.elapsed() > Duration::from_secs(5) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if finished.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let mut out = String::new();
    let _ = std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut out);
    let msg: String = out.lines().filter(|l| l.contains("panicked") || l.contains("Err") || l.contains("child opened")).collect::<Vec<_>>().join(" | ");
    let msg: String = msg.chars().take(400).collect();
    let elapsed = t.elapsed();
    assert!(finished.is_some(), "Vault::open on a folder with two `x -> .` symlinks was still running after 5 s (killed)");
    assert!(finished.unwrap().success(), "Vault::open failed after {elapsed:?}: {msg}");
}

// ---------------------------------------------------------------------------
// Deep nesting
// ---------------------------------------------------------------------------

#[test]
#[ignore = "not a defect (by design): notes nested deeper than 64 folders are silently invisible"]
fn fs20_notes_deeper_than_64_levels_are_invisible() {
    let d = tempfile::tempdir().unwrap();
    let mut p = d.path().to_path_buf();
    for i in 0..70 {
        p.push(format!("l{i}"));
    }
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("deep.md"), "deep").unwrap();
    let v = open(d.path());
    assert!(v.entries().iter().any(|e| e.path.ends_with("/deep.md")), "deep.md is not in the index");
}

#[test]
fn paths_beyond_path_max_are_skipped_without_failing_open() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("top.md"), "top").unwrap();
    let seg = "s".repeat(250);
    // 18 levels of 250-byte names (> 4096 bytes in total), built with relative cd.
    // cd -P because a logical cd in dash (/bin/sh on Debian and Ubuntu) chdirs to
    // the joined absolute path, which fails beyond PATH_MAX.
    let mk = format!("cd '{}' && for i in $(seq 1 18); do mkdir {seg} && cd -P {seg} || exit 1; done && echo deep > deep.md", d.path().display());
    assert!(std::process::Command::new("sh").arg("-c").arg(&mk).status().unwrap().success());
    let r = try_open(d.path());
    let rm = format!("cd '{}' && for i in $(seq 1 18); do cd -P {seg}; done; rm deep.md; for i in $(seq 1 18); do cd -P .. && rmdir {seg}; done", d.path().display());
    let _ = std::process::Command::new("sh").arg("-c").arg(&rm).status();
    let v = r.unwrap();
    assert!(v.index().note("top.md").is_some());
}

// ---------------------------------------------------------------------------
// Bulk changes
// ---------------------------------------------------------------------------

#[test]
fn bulk_5000_file_changes_are_complete_and_match_a_fresh_open() {
    let d = tempfile::tempdir().unwrap();
    let (nd, per) = (100, 50);
    for i in 0..nd {
        fs::create_dir_all(d.path().join(format!("dir{i}"))).unwrap();
        for j in 0..per {
            fs::write(d.path().join(format!("dir{i}/n{j}.md")), format!("note {i} {j} [[n{}]]", (j + 1) % per)).unwrap();
        }
    }
    let v = open(d.path());
    // 1. every folder renamed (like a script or a git checkout renaming folders)
    for i in 0..nd {
        fs::rename(d.path().join(format!("dir{i}")), d.path().join(format!("moved{i}"))).unwrap();
    }
    let c = v.rescan().unwrap();
    assert_eq!((c.len(), count(&c, is_renamed)), (nd, nd));
    assert_eq!(snapshot(&v), snapshot(&open(d.path())));
    // 2. every file renamed
    for i in 0..nd {
        for j in 0..per {
            fs::rename(d.path().join(format!("moved{i}/n{j}.md")), d.path().join(format!("moved{i}/r{j}.md"))).unwrap();
        }
    }
    let c = v.rescan().unwrap();
    assert_eq!((c.len(), count(&c, is_renamed)), (nd * per, nd * per));
    for ch in &c {
        if let Change::Renamed { from, entry } = ch {
            assert_eq!(from.replace("/n", "/r"), entry.path, "rename attributed to the wrong file");
        }
    }
    assert_eq!(snapshot(&v), snapshot(&open(d.path())));
    // 3. half deleted, as many new ones created
    for i in 0..nd {
        for j in 0..per / 2 {
            fs::remove_file(d.path().join(format!("moved{i}/r{j}.md"))).unwrap();
            fs::write(d.path().join(format!("moved{i}/new{j}.md")), format!("brand new {i} {j}")).unwrap();
        }
    }
    let c = v.rescan().unwrap();
    assert_eq!(count(&c, |c| matches!(c, Change::Deleted { .. })), nd * per / 2);
    assert_eq!(count(&c, |c| matches!(c, Change::Created { .. })), nd * per / 2);
    assert_eq!(snapshot(&v), snapshot(&open(d.path())));
    // 4. checkout: every folder replaced by a different folder with different files
    for i in 0..nd {
        fs::remove_dir_all(d.path().join(format!("moved{i}"))).unwrap();
        fs::create_dir_all(d.path().join(format!("other{i}"))).unwrap();
        for j in 0..per {
            fs::write(d.path().join(format!("other{i}/o{j}.md")), format!("other {i} {j}")).unwrap();
        }
    }
    v.rescan().unwrap();
    assert_eq!(snapshot(&v), snapshot(&open(d.path())));
    assert!(v.rescan().unwrap().is_empty());
}

/// `n` folders of 10 notes are replaced by `n` different folders (a `git
/// checkout` to a branch with another layout, unzipping a reorganized
/// archive, a sync tool applying a reorganization); returns rescan seconds.
fn replace_folders_secs(n: usize) -> (f64, usize) {
    let d = tempfile::tempdir().unwrap();
    for i in 0..n {
        fs::create_dir_all(d.path().join(format!("f{i:04}"))).unwrap();
        for j in 0..10 {
            fs::write(d.path().join(format!("f{i:04}/n{j}.md")), format!("{i} {j}")).unwrap();
        }
    }
    let v = open(d.path());
    for i in 0..n {
        fs::remove_dir_all(d.path().join(format!("f{i:04}"))).unwrap();
        fs::create_dir_all(d.path().join(format!("g{i:04}"))).unwrap();
        for j in 0..10 {
            fs::write(d.path().join(format!("g{i:04}/m{j}.md")), format!("other {i} {j}")).unwrap();
        }
    }
    let t = Instant::now();
    let c = v.rescan().unwrap();
    (t.elapsed().as_secs_f64(), c.len())
}

#[test]
fn fs12_replacing_many_folders_rescan_is_not_quadratic() {
    let (small, c1) = replace_folders_secs(50);
    let (big, c2) = replace_folders_secs(200);
    assert_eq!((c1, c2), (50 * 12, 200 * 12)); // n deleted folders, n created folders, 10n created notes
    let ratio = big / small;
    println!("50 folders: {small:.3} s, 200 folders: {big:.3} s, ratio {ratio:.1} (linear would be about 4)");
    assert!(ratio < 10.0, "4x the folders took {ratio:.1}x the time ({small:.3} s -> {big:.3} s)");
}

// ---------------------------------------------------------------------------
// Rename detection by content
// ---------------------------------------------------------------------------

#[test]
fn fs16_unrelated_identical_note_is_not_reported_as_rename() {
    let (d, v) = setup(&[("Meeting 2026-09-01.md", ""), ("work/keep.md", "k")]);
    // User deletes an empty note in one place and, separately, creates a new
    // empty note somewhere else before the next scan.
    fs::remove_file(d.path().join("Meeting 2026-09-01.md")).unwrap();
    fs::write(d.path().join("work/Todo.md"), "").unwrap();
    let c = v.rescan().unwrap();
    assert_eq!(count(&c, is_renamed), 0, "{c:?}");
}

#[test]
fn fs16_identical_notes_renamed_together_are_not_swapped() {
    let (d, v) = setup(&[("a.md", "template"), ("b.md", "template")]);
    fs::rename(d.path().join("a.md"), d.path().join("z-from-a.md")).unwrap();
    fs::rename(d.path().join("b.md"), d.path().join("y-from-b.md")).unwrap();
    let c = v.rescan().unwrap();
    for ch in &c {
        if let Change::Renamed { from, entry } = ch {
            let expect = if from == "a.md" { "z-from-a.md" } else { "y-from-b.md" };
            assert_eq!(entry.path, expect, "{from} reported as renamed to {}", entry.path);
        }
    }
}

#[test]
fn renames_of_distinct_notes_are_attributed_correctly() {
    let (d, v) = setup(&[("a.md", "alpha"), ("b.md", "beta"), ("pics/p.png", "png"), ("dir/x.md", "x")]);
    fs::rename(d.path().join("a.md"), d.path().join("b2.md")).unwrap();
    fs::rename(d.path().join("b.md"), d.path().join("a2.md")).unwrap();
    fs::rename(d.path().join("dir"), d.path().join("dir2")).unwrap();
    fs::rename(d.path().join("pics/p.png"), d.path().join("p.png")).unwrap();
    let mut got: Vec<(String, String)> = v
        .rescan()
        .unwrap()
        .into_iter()
        .filter_map(|c| match c {
            Change::Renamed { from, entry } => Some((from, entry.path)),
            _ => None,
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("a.md".to_string(), "b2.md".to_string()),
            ("b.md".into(), "a2.md".into()),
            ("dir".into(), "dir2".into()),
            ("pics/p.png".into(), "p.png".into())
        ]
    );
}

#[test]
fn fs17_replaced_folder_with_same_sizes_refreshes_index() {
    let (d, v) = setup(&[("t1.md", ""), ("old/x.md", "aaaa [[t1]]")]);
    let m = fs::metadata(d.path().join("old/x.md")).unwrap().modified().unwrap();
    // Folder deleted, a different one restored from an archive (mtimes preserved).
    fs::remove_dir_all(d.path().join("old")).unwrap();
    fs::create_dir_all(d.path().join("new")).unwrap();
    fs::write(d.path().join("new/x.md"), "bbbb [[t2]]").unwrap();
    fs::OpenOptions::new().write(true).open(d.path().join("new/x.md")).unwrap().set_modified(m).unwrap();
    let c = v.rescan().unwrap();
    let indexed = v.index().note("new/x.md").map(|n| n.content.clone());
    let disk = v.read_note("new/x.md").unwrap();
    let save = v.write_note("new/x.md", "bbbb [[t2]] edited", Some(&disk.hash));
    assert_eq!(indexed.as_deref(), Some("bbbb [[t2]]"), "index holds the deleted folder's text (changes: {c:?})");
    assert!(save.is_ok(), "save with the hash just read from disk: {save:?}");
}

// ---------------------------------------------------------------------------
// Encodings
// ---------------------------------------------------------------------------

#[test]
fn fs15_utf8_bom_does_not_hide_frontmatter() {
    let (_d, v) = setup(&[("bom.md", "\u{feff}---\ntags: [alpha]\ntitle: T\n---\n# Head\n"), ("plain.md", "---\ntags: [alpha]\ntitle: T\n---\n# Head\n")]);
    let plain = v.note_info("plain.md").unwrap();
    let bom = v.note_info("bom.md").unwrap();
    assert_eq!(plain.tags, vec!["alpha"]);
    assert_eq!(bom.tags, plain.tags, "tags of the BOM note");
    assert_eq!(bom.frontmatter, plain.frontmatter);
    assert_eq!(bom.headings.len(), plain.headings.len(), "headings: {:?}", bom.headings);
}

// ---------------------------------------------------------------------------
// Very large notes
// ---------------------------------------------------------------------------

/// A note of `n` list items, each with one wikilink and one tag (a daily log).
fn log_note(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("- item {i} [[note{}]] #tag{}\n", i % 100, i % 10));
    }
    s
}

#[test]
fn fs11_indexing_time_is_not_quadratic_in_links_and_tags() {
    let (_d, v) = setup(&[]);
    let small = log_note(2_000); // ~55 KB
    let big = log_note(16_000); // ~450 KB
    let t = Instant::now();
    v.write_note("small.md", &small, None).unwrap();
    let ts = t.elapsed().as_secs_f64();
    let t = Instant::now();
    v.write_note("big.md", &big, None).unwrap();
    let tb = t.elapsed().as_secs_f64();
    let ratio = tb / ts;
    println!("2k lines: {ts:.3} s, 16k lines: {tb:.3} s, ratio {ratio:.1} (linear would be about 8)");
    assert!(ratio < 16.0, "8x the content took {ratio:.1}x the time ({ts:.3} s -> {tb:.3} s)");
}

#[test]
#[ignore = "slow: 60 MB note; open, search and save without panics (prints timings)"]
fn huge_note_opens_searches_and_saves() {
    let d = tempfile::tempdir().unwrap();
    let mut s = String::with_capacity(61 << 20);
    let mut i = 0u64;
    while s.len() < 60 << 20 {
        s.push_str(&format!("Line {i} lorem ipsum dolor sit amet word{i}\n"));
        i += 1;
    }
    fs::write(d.path().join("huge.md"), &s).unwrap();
    fs::write(d.path().join("oneline.md"), "x".repeat(50 << 20)).unwrap();
    fs::write(d.path().join("small.md"), "[[huge]] tiny").unwrap();
    let t = Instant::now();
    let v = open(d.path());
    println!("open: {:?}", t.elapsed());
    assert_eq!(v.index().note_count(), 3);
    let t = Instant::now();
    assert_eq!(v.search("word123456", 10).len(), 1);
    println!("search: {:?}", t.elapsed());
    assert_eq!(v.backlinks("huge.md").len(), 1);
    let n = v.read_note("huge.md").unwrap();
    let t = Instant::now();
    v.write_note("huge.md", &format!("{}more\n", n.content), Some(&n.hash)).unwrap();
    println!("save: {:?}", t.elapsed());
    assert!(v.rescan().unwrap().is_empty());
}

#[test]
fn rescan_paths_ignores_hidden_hints_and_handles_vanished_roots() {
    let (d, v) = setup(&[("a/x.md", "x"), ("b.md", "b")]);
    fs::create_dir_all(d.path().join(".cairn")).unwrap();
    fs::write(d.path().join(".cairn/settings.json"), "{}").unwrap();
    assert!(v.rescan_paths(&[".cairn/settings.json".into(), ".git".into()]).unwrap().is_empty());
    fs::remove_dir_all(d.path().join("a")).unwrap();
    let c = v.rescan_paths(&["a/x.md".into(), "a".into()]).unwrap();
    assert_eq!(c, vec![Change::Deleted { path: "a".into(), kind: cairn_core::EntryKind::Dir }]);
    assert!(matches!(v.rescan_paths(&["../etc".into()]), Ok(c) if c.is_empty()));
    assert!(v.rescan().unwrap().is_empty());
    let _ = CoreError::NotFound(String::new());
}
