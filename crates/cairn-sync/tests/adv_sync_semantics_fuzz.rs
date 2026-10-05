//! Randomized three-device sync with richer operations than the two-device
//! fuzz in two_devices.rs: nested folders, folder renames, case-only
//! renames, moves into and out of a hidden folder, binary files, empty files,
//! identical-content files, delete + recreate, CRLF lines, inserts in the
//! middle of notes, and mtime changes far into the past and future. Every
//! other rename is made in the app, which records it for the next sync.
//!
//! Invariants checked at the end of every run:
//! 1. every sync succeeds;
//! 2. after syncing until nobody uploads anything, all three devices have
//!    identical visible trees (paths and bytes);
//! 3. every token any device ever wrote is still somewhere: in a visible
//!    file, a conflict copy, a hidden folder, the vault trash, or the copy a
//!    "user delete" kept aside.
//!
//! Environment (all optional):
//!   CAIRN_SS_FUZZ_SEEDS   number of seeds (default 6)
//!   CAIRN_SS_FUZZ_START   first seed (default 0)
//!   CAIRN_SS_FUZZ_STEPS   steps per seed (default 160)
//!   CAIRN_SS_FUZZ_THREADS worker threads (default 2)
//!   CAIRN_SS_FUZZ_LOG=1   print the operation log of failing seeds (=all: every seed)
//!   CAIRN_SS_FUZZ_STRICT=1 overlapping mode also fails on edits left only in a trash
//!
//!   cargo test -p cairn-sync --test adv_sync_semantics_fuzz
//!   CAIRN_SS_FUZZ_SEEDS=300 CAIRN_SS_FUZZ_THREADS=4 cargo test -p cairn-sync --test adv_sync_semantics_fuzz -- --nocapture
//!   CAIRN_SS_FUZZ_SEEDS=300 cargo test -p cairn-sync --test adv_sync_semantics_fuzz -- --ignored --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use common::*;
use parking_lot::Mutex;

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

#[derive(Clone, Copy)]
struct Mode {
    /// Let another device sync in the middle of a sync (between pull and
    /// push), and let the "user" edit a file while a sync runs.
    interleave: bool,
    /// Also fail when a written token survives only in a vault trash.
    strict_trash: bool,
}

/// Visible folders (relative paths) of a device.
fn dirs(root: &Path) -> Vec<String> {
    fn rec(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if e.path().is_dir() {
                out.push(e.path().strip_prefix(root).unwrap().to_string_lossy().to_string());
                rec(root, &e.path(), out);
            }
        }
    }
    let mut v = Vec::new();
    rec(root, root, &mut v);
    v.sort();
    v
}

fn hidden_files(root: &Path) -> Vec<String> {
    let a = root.join(".archive");
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(&a) {
        for e in rd.flatten() {
            v.push(format!(".archive/{}", e.file_name().to_string_lossy()));
        }
    }
    v.sort();
    v
}

const FOLDERS: &[&str] = &["", "", "f0", "f1", "f0/sub", "f1/deep/er", "Notes"];

/// A note no step touches: a vault folder with no files at all does not
/// sync (FINDING-006), and deleting every note is not what this checks.
const ANCHOR: &str = "anchor.md";

fn run_seed(seed: u64, steps: usize, mode: Mode) -> Result<(), String> {
    let srv = server();
    let archive = tempfile::tempdir().unwrap();
    let names = ["laptop", "phone", "tablet"];
    let devs: Vec<Arc<Mutex<Device>>> = names.iter().map(|n| Arc::new(Mutex::new(Device::new(&srv, n, &[])))).collect();
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03);
    let written: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut log: Vec<String> = Vec::new();
    let mut archived = 0usize;
    devs[0].lock().write(ANCHOR, "untouched\n");
    for d in &devs {
        d.lock().sync();
    }

    for step in 0..steps {
        let di = rng.below(3) as usize;
        let tok = format!("T{seed}x{step}d{di}");
        let op = rng.below(100);
        let d = devs[di].clone();
        let dl = d.lock();
        let files: Vec<String> = dl.paths().into_iter().filter(|p| p != ANCHOR).collect();
        let text_files: Vec<String> = files.iter().filter(|p| p.ends_with(".md") || p.ends_with(".txt")).cloned().collect();
        let desc: String;
        // Renames on every other step are made in the app (without drawing
        // from `rng`, so that they do not change the other steps of a seed).
        let in_app = (seed + step as u64).is_multiple_of(2);
        let mv = |a: &str, b: &str| if in_app { dl.app_mv(a, b) } else { dl.mv(a, b) };
        let how = if in_app { " in the app" } else { "" };
        match op {
            // create a note (sometimes a case variant of an existing name)
            0..=13 => {
                let folder = *rng.pick(FOLDERS);
                let stem = if rng.chance(15) { format!("N{}", rng.below(6)) } else { format!("n{}", rng.below(10)) };
                let ext = if rng.chance(10) { "txt" } else { "md" };
                let p = if folder.is_empty() { format!("{stem}.{ext}") } else { format!("{folder}/{stem}.{ext}") };
                if dl.exists(&p) || dirs(&dl.root).contains(&p) {
                    desc = format!("skip create {p}");
                } else {
                    let body = if rng.chance(20) { format!("{tok}\r\n") } else { format!("{tok}\n") };
                    dl.write(&p, &body);
                    written.lock().push(tok.clone());
                    desc = format!("create {p}");
                }
            }
            // edit: append or insert a line somewhere in a text file
            14..=33 if !text_files.is_empty() => {
                let p = rng.pick(&text_files).clone();
                let old = dl.read(&p).unwrap_or_default();
                let mut lines: Vec<&str> = old.split_inclusive('\n').collect();
                let pos = rng.below(lines.len() as u64 + 1) as usize;
                let line = format!("{tok}\n");
                lines.insert(pos, &line);
                dl.write(&p, &lines.concat());
                written.lock().push(tok.clone());
                desc = format!("edit {p} at line {pos}");
            }
            // user delete (keep a copy aside, like a trash)
            34..=38 if !files.is_empty() => {
                let p = rng.pick(&files).clone();
                archived += 1;
                fs::copy(dl.abs(&p), archive.path().join(format!("{archived}"))).unwrap();
                dl.rm(&p);
                desc = format!("delete {p}");
            }
            // rename / move a file
            39..=46 if !files.is_empty() => {
                let p = rng.pick(&files).clone();
                let folder = *rng.pick(FOLDERS);
                let ext = p.rsplit('.').next().unwrap_or("md").to_string();
                let name = format!("m{}.{ext}", rng.below(12));
                let to = if folder.is_empty() { name } else { format!("{folder}/{name}") };
                if dl.exists(&to) || dirs(&dl.root).contains(&to) {
                    desc = format!("skip rename {p} -> {to}");
                } else {
                    mv(&p, &to);
                    desc = format!("rename {p} -> {to}{how}");
                }
            }
            // case-only rename
            47..=49 if !files.is_empty() => {
                let p = rng.pick(&files).clone();
                let (dir, name) = match p.rfind('/') {
                    Some(i) => (&p[..i + 1], &p[i + 1..]),
                    None => ("", p.as_str()),
                };
                let mut cs = name.chars();
                let first = cs.next().unwrap();
                let flipped = if first.is_uppercase() { first.to_lowercase().collect::<String>() } else { first.to_uppercase().collect() };
                let to = format!("{dir}{flipped}{}", cs.as_str());
                if to == p || dl.exists(&to) {
                    desc = format!("skip case rename {p} -> {to}");
                } else {
                    mv(&p, &to);
                    desc = format!("case rename {p} -> {to}{how}");
                }
            }
            // folder rename / move
            50..=54 => {
                let ds = dirs(&dl.root);
                if ds.is_empty() {
                    desc = "skip folder rename (no folders)".into();
                } else {
                    let from = rng.pick(&ds).clone();
                    let to = match rng.below(3) {
                        0 => format!("g{}", rng.below(4)),
                        1 => format!("g{}/inner{}", rng.below(3), rng.below(3)),
                        _ => format!("{from}x"),
                    };
                    if dl.exists(&to) || to.starts_with(&format!("{from}/")) || files.iter().any(|f| f == &to) {
                        desc = format!("skip folder rename {from} -> {to}");
                    } else {
                        mv(&from, &to);
                        desc = format!("folder rename {from} -> {to}{how}");
                    }
                }
            }
            // move into the hidden .archive folder
            55..=57 if !files.is_empty() => {
                let p = rng.pick(&files).clone();
                let to = format!(".archive/{step}-{}", p.replace('/', "_"));
                dl.mv(&p, &to);
                desc = format!("hide {p} -> {to}");
            }
            // move back out of the hidden folder
            58..=59 => {
                let hs = hidden_files(&dl.root);
                if hs.is_empty() {
                    desc = "skip unhide".into();
                } else {
                    let h = rng.pick(&hs).clone();
                    let name = h.trim_start_matches(".archive/");
                    let to = format!("back/{name}");
                    if dl.exists(&to) {
                        desc = format!("skip unhide {h}");
                    } else {
                        dl.mv(&h, &to);
                        desc = format!("unhide {h} -> {to}");
                    }
                }
            }
            // binary file: create or append a token
            60..=63 => {
                let bins: Vec<String> = files.iter().filter(|p| p.ends_with(".png")).cloned().collect();
                if !bins.is_empty() && rng.chance(50) {
                    let p = rng.pick(&bins).clone();
                    let mut b = fs::read(dl.abs(&p)).unwrap();
                    b.extend_from_slice(b"\x00\xff");
                    b.extend_from_slice(tok.as_bytes());
                    b.push(b'\n');
                    dl.write_bytes(&p, &b);
                    written.lock().push(tok.clone());
                    desc = format!("binary append {p}");
                } else {
                    let folder = *rng.pick(FOLDERS);
                    let name = format!("img{}.png", rng.below(5));
                    let p = if folder.is_empty() { name } else { format!("{folder}/{name}") };
                    if dl.exists(&p) || dirs(&dl.root).contains(&p) {
                        desc = format!("skip binary {p}");
                    } else {
                        let mut b = vec![0x89, b'P', b'N', b'G', 0xff, 0xfe, 0x00];
                        b.extend_from_slice(tok.as_bytes());
                        b.push(b'\n');
                        dl.write_bytes(&p, &b);
                        written.lock().push(tok.clone());
                        desc = format!("binary create {p}");
                    }
                }
            }
            // empty file
            64..=66 => {
                let folder = *rng.pick(FOLDERS);
                let name = format!("e{}.md", rng.below(4));
                let p = if folder.is_empty() { name } else { format!("{folder}/{name}") };
                if dl.exists(&p) || dirs(&dl.root).contains(&p) {
                    desc = format!("skip empty {p}");
                } else {
                    dl.write(&p, "");
                    desc = format!("empty {p}");
                }
            }
            // identical copy of an existing file
            67..=69 if !files.is_empty() => {
                let p = rng.pick(&files).clone();
                let ext = p.rsplit('.').next().unwrap_or("md").to_string();
                let to = format!("copies/c{}.{ext}", rng.below(6));
                if dl.exists(&to) {
                    desc = format!("skip copy {p}");
                } else {
                    let b = fs::read(dl.abs(&p)).unwrap();
                    dl.write_bytes(&to, &b);
                    desc = format!("copy {p} -> {to}");
                }
            }
            // mtime jumps (touch into the far past / future / now)
            70..=72 if !files.is_empty() => {
                let p = rng.pick(&files).clone();
                let t = match rng.below(3) {
                    0 => UNIX_EPOCH + Duration::from_secs(86_400 * 30),
                    1 => UNIX_EPOCH + Duration::from_secs(13_000_000_000),
                    _ => std::time::SystemTime::now() - Duration::from_secs(rng.below(1_000_000)),
                };
                set_mtime(&dl.abs(&p), t);
                desc = format!("touch {p}");
            }
            // delete and recreate at the same path
            73..=75 if !text_files.is_empty() => {
                let p = rng.pick(&text_files).clone();
                archived += 1;
                fs::copy(dl.abs(&p), archive.path().join(format!("{archived}"))).unwrap();
                dl.rm(&p);
                dl.write(&p, &format!("{tok}\n"));
                written.lock().push(tok.clone());
                desc = format!("recreate {p}");
            }
            // sync
            _ => {
                drop(dl);
                let mut extra = String::new();
                if mode.interleave && rng.chance(35) {
                    let ei = (di + 1 + rng.below(2) as usize) % 3;
                    let e = devs[ei].clone();
                    let hook: Hook = Box::new(move || {
                        let _ = e.lock().try_sync();
                    });
                    if rng.chance(50) {
                        *d.lock().hooks.before_put.lock() = Some(hook);
                        extra = format!(" (+{} syncs before first upload)", names[ei]);
                    } else {
                        *d.lock().hooks.after_changes.lock() = Some(hook);
                        extra = format!(" (+{} syncs after the fetch)", names[ei]);
                    }
                }
                if mode.interleave && rng.chance(15) {
                    // the user saves a note while the sync runs
                    let dl = d.lock();
                    let tf: Vec<String> = dl.paths().into_iter().filter(|p| p.ends_with(".md") && p != ANCHOR).collect();
                    if !tf.is_empty() {
                        let p = rng.pick(&tf).clone();
                        let abs = dl.abs(&p);
                        let w = written.clone();
                        let t2 = format!("{tok}u");
                        let prev = dl.hooks.after_changes.lock().take();
                        *dl.hooks.after_changes.lock() = Some(Box::new(move || {
                            if let Some(h) = prev {
                                h();
                            }
                            std::thread::sleep(Duration::from_millis(3));
                            let old = fs::read_to_string(&abs).unwrap_or_default();
                            if fs::write(&abs, format!("{old}{t2}\n")).is_ok() {
                                w.lock().push(t2);
                            }
                        }));
                        extra.push_str(&format!(" (+user edits {p} during sync)"));
                    }
                }
                let r = d.lock().try_sync();
                // hooks that did not fire must not leak into later syncs
                *d.lock().hooks.before_put.lock() = None;
                *d.lock().hooks.after_changes.lock() = None;
                match r {
                    Ok(r) => desc = format!("sync{extra}: rounds {} pulled {} pushed {} conflicts {:?}", r.rounds, r.pulled, r.pushed, r.conflicts),
                    Err(e) => {
                        log.push(format!("{step}: {} sync{extra} FAILED: {e}", names[di]));
                        return Err(format!("seed {seed}: sync on {} failed at step {step}: {e}\n{}", names[di], fmt_log(&log)));
                    }
                }
                log.push(format!("{step}: {} {desc}", names[di]));
                continue;
            }
        }
        log.push(format!("{step}: {} {desc}", names[di]));
    }

    // final convergence
    let mut guards: Vec<_> = devs.iter().map(|d| d.lock()).collect();
    let mut refs: Vec<&mut Device> = guards.iter_mut().map(|g| &mut **g).collect();
    if let Err(e) = try_converge(&mut refs) {
        return Err(format!("seed {seed}: {e}\n{}", fmt_log(&log)));
    }
    let mut everywhere = String::new();
    for d in refs.iter() {
        everywhere.push_str(&d.all_text());
    }
    let mut v = Vec::new();
    walk(archive.path(), archive.path(), true, &mut v);
    for (_, b) in v {
        everywhere.push_str(&String::from_utf8_lossy(&b));
    }
    let lost: Vec<String> = written.lock().iter().filter(|t| !t.is_empty() && !everywhere.contains(t.as_str())).cloned().collect();
    if !lost.is_empty() {
        return Err(format!("seed {seed}: tokens lost by sync: {lost:?}\n{}", fmt_log(&log)));
    }
    // Text that survives only in a trash was removed by sync, not by a
    // user (user deletes are kept in `archive`): an edit lost to a delete.
    if mode.strict_trash || std::env::var("CAIRN_SS_FUZZ_STRICT").is_ok() {
        let mut outside = String::new();
        for d in refs.iter() {
            outside.push_str(&d.non_trash_text());
        }
        let mut v = Vec::new();
        walk(archive.path(), archive.path(), true, &mut v);
        for (_, b) in v {
            outside.push_str(&String::from_utf8_lossy(&b));
        }
        let trashed: Vec<String> = written.lock().iter().filter(|t| !t.is_empty() && !outside.contains(t.as_str())).cloned().collect();
        if !trashed.is_empty() {
            return Err(format!("seed {seed}: edits only left in a trash (sync deleted them): {trashed:?}\n{}", fmt_log(&log)));
        }
    }
    if std::env::var("CAIRN_SS_FUZZ_LOG").as_deref() == Ok("all") {
        eprintln!("seed {seed} ok:\n{}\nfinal tree: {:?}\n", log.join("\n"), refs[0].paths());
    }
    Ok(())
}

fn fmt_log(log: &[String]) -> String {
    if std::env::var("CAIRN_SS_FUZZ_LOG").is_ok() {
        log.join("\n")
    } else {
        format!("({} steps; set CAIRN_SS_FUZZ_LOG=1 for the operation log)", log.len())
    }
}

fn run_many(mode: Mode, default_seeds: u64) {
    let seeds = env_u64("CAIRN_SS_FUZZ_SEEDS", default_seeds);
    let start = env_u64("CAIRN_SS_FUZZ_START", 0);
    let steps = env_u64("CAIRN_SS_FUZZ_STEPS", 160) as usize;
    let threads = env_u64("CAIRN_SS_FUZZ_THREADS", 2).max(1);
    let next = Arc::new(std::sync::atomic::AtomicU64::new(start));
    let failures: Arc<Mutex<Vec<(u64, String)>>> = Arc::new(Mutex::new(Vec::new()));
    std::thread::scope(|s| {
        for _ in 0..threads {
            let next = next.clone();
            let failures = failures.clone();
            s.spawn(move || {
                loop {
                    let seed = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if seed >= start + seeds {
                        break;
                    }
                    let r = std::panic::catch_unwind(|| run_seed(seed, steps, mode));
                    let r = match r {
                        Ok(r) => r,
                        Err(p) => Err(format!(
                            "seed {seed}: panic: {}",
                            p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
                        )),
                    };
                    if let Err(e) = r {
                        eprintln!("FAIL {e}\n");
                        failures.lock().push((seed, e));
                    }
                }
            });
        }
    });
    let mut f = failures.lock().clone();
    f.sort_by_key(|x| x.0);
    let ids: Vec<u64> = f.iter().map(|x| x.0).collect();
    eprintln!("{} of {seeds} seeds failed: {ids:?}", f.len());
    assert!(f.is_empty(), "{} of {seeds} seeds failed: {ids:?}\nfirst: {}", f.len(), f[0].1);
}

/// Sequential syncs (one device at a time, no sync overlaps another).
#[test]
fn randomized_three_device_rich_ops() {
    run_many(Mode { interleave: false, strict_trash: true }, 6);
}

/// Same, but syncs overlap: another device syncs between this device's
/// fetch and its uploads, and the user saves notes while a sync runs.
#[test]
fn randomized_three_device_rich_ops_overlapping_syncs() {
    run_many(Mode { interleave: true, strict_trash: false }, 40);
}
