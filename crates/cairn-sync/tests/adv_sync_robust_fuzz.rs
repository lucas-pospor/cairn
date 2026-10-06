//! Randomized sync robustness, on seeds the existing two_devices fuzz never
//! uses (it runs seeds 0..CAIRN_FUZZ_SEEDS). Three devices, more kinds of
//! operations (rename+edit, folder renames, attachments), and optionally
//! injected network errors, lost responses and crashes with restarts.
//!
//!   SR_FUZZ_FROM   first seed (default 1000)
//!   SR_FUZZ_SEEDS  number of seeds (default 6, keeps the run short)
//!   SR_FUZZ_STEPS  operations per seed (default 150)
//!   SR_FUZZ_STRICT also fail on seeds that break the one-file-id-per-path
//!                  invariant of the sync state (FINDING-145)
//!   SR_FUZZ_TRACE  print every operation and sync; SR_FUZZ_FSLOG=<text> also
//!                  prints vault writes/renames on paths containing <text>
//!
//! Run: cargo test -p cairn-sync --test adv_sync_robust_fuzz
//! Many seeds: SR_FUZZ_SEEDS=300 cargo test -p cairn-sync --test adv_sync_robust_fuzz -- --include-ignored

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::sync::{Arc, Mutex};

use common::*;

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[derive(Clone, Copy, PartialEq)]
enum Faults {
    None,
    Network,
}

/// How the devices pull.
#[derive(Clone, Copy, PartialEq)]
enum Pull {
    /// In one batch (a vault smaller than 64 MB).
    Whole,
    /// Each record in a batch of its own (a large vault comes in batches of
    /// about 64 MB).
    SmallBatches,
    /// As `SmallBatches`, and on about half of the syncs another device
    /// edits or renames a file and syncs while the pull is between two
    /// batches, so heads move under it.
    SmallBatchesWithChanges,
}

struct Outcome {
    /// two live file ids tracking one path (seen after a step's sync)
    invariant: Vec<String>,
    lost: Vec<String>,
    diverged: Option<String>,
    crashes: usize,
    errors: usize,
}

fn install_faults(d: &mut Device, rng: &mut Rng) {
    let mut r1 = Rng(rng.next() | 1);
    let t = FaultTransport::new(
        http(&d.url),
        Box::new(move |_, _| match r1.below(100) {
            0..=2 => Fault::ErrBefore,
            3..=5 => Fault::ErrAfter,
            6..=7 => Fault::PanicAfter,
            8 => Fault::PanicBefore,
            _ => Fault::None,
        }),
    );
    d.set_transport(Box::new(t));
    let mut r2 = Rng(rng.next() | 1);
    *d.hookfs.decide.lock() = Some(Box::new(move |_, _, _| match r2.below(100) {
        0..=1 => FsAction::PanicAfter,
        2 => FsAction::PanicBefore,
        _ => FsAction::Pass,
    }));
}

fn clear_faults(d: &mut Device) {
    *d.hookfs.decide.lock() = None;
    let url = d.url.clone();
    d.set_transport(http(&url));
}

/// A note no step touches, for the sweeps: a vault folder with no files at
/// all does not sync (FINDING-006), and deleting every note is not what they
/// check. The pinned seeds below run without it, as they were recorded.
const ANCHOR: &str = "anchor.md";

fn run(seed: u64, faults: Faults, steps: u64, anchor: bool, pull: Pull) -> Outcome {
    quiet_simulated_crashes();
    let srv = server();
    let archive = tempfile::tempdir().unwrap();
    let mut archived = 0;
    let mut devs = vec![Device::new(&srv, "laptop", &[]), Device::new(&srv, "phone", &[]), Device::new(&srv, "tablet", &[])];
    if pull != Pull::Whole {
        for d in devs.iter_mut() {
            d.pull_in_batches(1, 1);
        }
    }
    if anchor {
        devs[0].write(ANCHOR, "untouched\n");
        for d in devs.iter_mut() {
            d.sync_ok();
        }
    }
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut written: Vec<String> = Vec::new();
    let mut crashes = 0;
    let mut errors = 0;
    let mut invariant: Vec<String> = Vec::new();
    let trace = std::env::var("SR_FUZZ_TRACE").is_ok();
    if let Ok(pat) = std::env::var("SR_FUZZ_FSLOG") {
        for d in devs.iter() {
            let name = d.name.clone();
            let pat = pat.clone();
            *d.hookfs.decide.lock() = Some(Box::new(move |op, path, _| {
                if path.contains(&pat) {
                    eprintln!("    fs {name}: {op:?} {path}");
                }
                FsAction::Pass
            }));
        }
    }
    for step in 0..steps {
        let d = rng.below(devs.len() as u64) as usize;
        if trace && std::env::var("SR_FUZZ_TRACE_FILES").is_ok() {
            eprintln!("--- step {step} on {} files {:?}", devs[d].name, devs[d].files());
        }
        let files: Vec<String> = devs[d].paths().into_iter().filter(|p| p != ANCHOR).collect();
        let pick = |rng: &mut Rng| files[rng.below(files.len() as u64) as usize].clone();
        let op = rng.below(14);
        if trace {
            eprintln!("  step {step} dev {} op {op}", devs[d].name);
        }
        match op {
            0..=2 => {
                let p = format!("f{}/n{}.md", rng.below(3), rng.below(12));
                if !devs[d].root.join(&p).exists() {
                    let line = format!("created {step} on {d}\n");
                    devs[d].write(&p, &line);
                    written.push(line);
                }
            }
            3..=5 if !files.is_empty() => {
                let p = pick(&mut rng);
                if trace {
                    eprintln!("    edit {p}");
                }
                let old = devs[d].read(&p).unwrap_or_default();
                let line = format!("edit {step} on {d}\n");
                devs[d].write(&p, &format!("{old}{line}"));
                written.push(line);
            }
            6 if !files.is_empty() => {
                let p = pick(&mut rng);
                if trace {
                    eprintln!("    rm {p}");
                }
                archived += 1;
                fs::copy(devs[d].root.join(&p), archive.path().join(format!("{archived}.txt"))).unwrap();
                devs[d].rm(&p);
            }
            7 if !files.is_empty() => {
                let p = pick(&mut rng);
                let ext = if p.ends_with(".bin") { "bin" } else { "md" };
                let to = format!("r{}/m{}.{ext}", rng.below(2), rng.below(20));
                if !devs[d].root.join(&to).exists() {
                    if trace {
                        eprintln!("    mv {p} -> {to}");
                    }
                    devs[d].mv(&p, &to);
                }
            }
            8 if !files.is_empty() => {
                // rename and keep typing before the next sync
                let p = pick(&mut rng);
                let ext = if p.ends_with(".bin") { "bin" } else { "md" };
                let to = format!("r{}/m{}.{ext}", rng.below(2), rng.below(20));
                if !devs[d].root.join(&to).exists() {
                    let old = devs[d].read(&p).unwrap_or_default();
                    devs[d].mv(&p, &to);
                    let line = format!("renedit {step} on {d}\n");
                    devs[d].write(&to, &format!("{old}{line}"));
                    written.push(line);
                }
            }
            9 => {
                // an attachment (not mergeable): conflicts become copies
                let p = format!("att/a{}.bin", rng.below(3));
                if trace {
                    eprintln!("    append {p}");
                }
                let old = devs[d].read(&p).unwrap_or_default();
                let line = format!("bin {step} on {d}\n");
                devs[d].write(&p, &format!("{old}{line}"));
                written.push(line);
            }
            10 => {
                let from = format!("f{}", rng.below(3));
                let to = format!("g{}", rng.below(3));
                let (a, b) = (devs[d].root.join(&from), devs[d].root.join(&to));
                if a.is_dir() && !b.exists() {
                    fs::rename(a, b).unwrap();
                }
            }
            _ => {
                if faults == Faults::Network {
                    install_faults(&mut devs[d], &mut rng);
                }
                let meddler = (pull == Pull::SmallBatchesWithChanges && rng.chance(50)).then(|| meddle(&mut devs, d, step, &mut rng));
                let res = devs[d].sync_may_crash();
                if let Some((o, slot)) = meddler {
                    // the other device back, whether the hook ran or not,
                    // and a transport without the hook
                    let (back, lines) = slot.lock().unwrap().take().unwrap();
                    devs[o] = back;
                    written.extend(lines);
                    devs[d].pull_in_batches(1, 1);
                }
                // invariant: one live file id per path in the sync state
                if let Some(e) = devs[d].engine.as_ref() {
                    let mut seen = std::collections::HashMap::new();
                    for (fid, t) in &e.state().files {
                        if !t.deleted
                            && let Some(other) = seen.insert(t.path.clone(), fid.clone())
                        {
                            let msg = format!("{} after step {step}: {other} and {fid} both track {}", devs[d].name, t.path);
                            if trace {
                                eprintln!("    INVARIANT BROKEN on {msg}");
                            }
                            if invariant.len() < 3 {
                                invariant.push(msg);
                            }
                        }
                    }
                }
                if trace {
                    eprintln!("    sync {} -> {:?}", devs[d].name, res.as_ref().map(|r| r.as_ref().map(|r| (r.pulled, r.pushed, r.conflicts.clone())).map_err(|e| e.to_string())));
                }
                match res {
                    None => {
                        crashes += 1;
                        devs[d].restart();
                    }
                    Some(Err(_)) => errors += 1,
                    Some(Ok(_)) => {}
                }
                if faults == Faults::Network && devs[d].engine.is_some() {
                    clear_faults(&mut devs[d]);
                }
            }
        }
    }
    // settle without faults
    let mut diverged = None;
    for _ in 0..3 {
        for d in devs.iter_mut() {
            let r = d.sync();
            if trace {
                eprintln!("    final sync {} -> {:?}", d.name, r.as_ref().map(|r| (r.pulled, r.pushed, r.conflicts.clone())).map_err(|e| e.to_string()));
            }
            if let Err(e) = r {
                if trace {
                    eprintln!("    {} files: {:?}", d.name, d.paths());
                    eprintln!("    {} state: {:?}", d.name, d.engine.as_ref().map(|e| e.state().files.values().map(|t| (t.path.clone(), t.seq, t.deleted)).collect::<Vec<_>>()));
                }
                diverged = Some(format!("final sync on {} failed: {e}", d.name));
            }
        }
    }
    let f0 = devs[0].files();
    for d in &devs[1..] {
        let f = d.files();
        if f != f0 && diverged.is_none() {
            let only0: Vec<&String> = f0.iter().filter(|x| !f.contains(x)).map(|x| &x.0).collect();
            let only1: Vec<&String> = f.iter().filter(|x| !f0.contains(x)).map(|x| &x.0).collect();
            diverged = Some(format!("{} vs {}: only on {}: {only0:?}; only on {}: {only1:?}", devs[0].name, d.name, devs[0].name, d.name));
        }
    }
    let mut everywhere: String = devs.iter().map(|d| d.all_text()).collect();
    let mut v = Vec::new();
    walk(archive.path(), archive.path(), &mut v);
    for (_, c) in v {
        everywhere.push_str(&c);
    }
    let lost = written.into_iter().filter(|l| !everywhere.contains(l.as_str())).collect();
    Outcome { invariant, lost, diverged, crashes, errors }
}

/// The other device and what it wrote, while `meddle`'s hook has it.
type Meddler = Arc<Mutex<Option<(Device, Vec<String>)>>>;

/// Before device `d` reads page 2, 3 or 4 of the feed, another device edits
/// or renames one of its files and syncs. That device is moved out of
/// `devs` (a stub is left in its place) until the caller takes it back.
fn meddle(devs: &mut [Device], d: usize, step: u64, rng: &mut Rng) -> (usize, Meddler) {
    let o = (d + 1 + rng.below(devs.len() as u64 - 1) as usize) % devs.len();
    let stub = Device::stub(&devs[o].root.clone(), devs[o].vault.clone(), "stub");
    let slot: Meddler = Arc::new(Mutex::new(Some((std::mem::replace(&mut devs[o], stub), Vec::new()))));
    let (at, mut r) = (2 + rng.below(3), Rng(rng.next() | 1));
    let (hook_slot, mut calls) = (slot.clone(), 0);
    let t = devs[d].pull_in_batches(1, 1);
    *t.before.lock() = Some(Box::new(move |op, _| {
        if op != "changes" {
            return;
        }
        calls += 1;
        if calls != at {
            return;
        }
        let mut g = hook_slot.lock().unwrap();
        let (dev, lines) = g.as_mut().unwrap();
        let files: Vec<String> = dev.paths().into_iter().filter(|p| p != ANCHOR).collect();
        if !files.is_empty() {
            let p = files[r.below(files.len() as u64) as usize].clone();
            if r.chance(25) {
                let ext = if p.ends_with(".bin") { "bin" } else { "md" };
                let to = format!("r{}/m{}.{ext}", r.below(2), r.below(20));
                if !dev.root.join(&to).exists() {
                    dev.mv(&p, &to);
                }
            } else {
                let line = format!("meddle {step} on {o}\n");
                let old = dev.read(&p).unwrap_or_default();
                dev.write(&p, &format!("{old}{line}"));
                lines.push(line);
            }
        }
        let _ = dev.sync();
    }));
    (o, slot)
}

fn sweep(faults: Faults, check_converge: bool, pull: Pull) {
    let from = env("SR_FUZZ_FROM", 1000);
    let n = env("SR_FUZZ_SEEDS", 6);
    let steps = env("SR_FUZZ_STEPS", 150);
    let started = std::time::Instant::now();
    let mut bad = Vec::new();
    let (mut crashes, mut errors) = (0, 0);
    let mut invariant_seeds: Vec<String> = Vec::new();
    for seed in from..from + n {
        let o = run(seed, faults, steps, true, pull);
        crashes += o.crashes;
        errors += o.errors;
        if !o.lost.is_empty() {
            bad.push(format!("seed {seed}: lines lost: {:?}", o.lost));
        }
        if !o.invariant.is_empty() {
            invariant_seeds.push(format!("seed {seed}: {}", o.invariant[0]));
        }
        if check_converge
            && let Some(d) = o.diverged
        {
            bad.push(format!("seed {seed}: {d}"));
        }
    }
    eprintln!(
        "fuzz {}{}: seeds {from}..{} ({steps} steps), {crashes} simulated crashes, {errors} failed syncs, {} bad seeds, {:?}",
        if faults == Faults::None { "no faults" } else { "with faults" },
        match pull {
            Pull::Whole => "",
            Pull::SmallBatches => ", small batches",
            Pull::SmallBatchesWithChanges => ", small batches with changes during the pull",
        },
        from + n,
        bad.len(),
        started.elapsed()
    );
    if !invariant_seeds.is_empty() {
        eprintln!("{} seeds broke the one-file-id-per-path invariant (FINDING-145):\n{}", invariant_seeds.len(), invariant_seeds.join("\n"));
    }
    if std::env::var("SR_FUZZ_STRICT").is_ok() {
        bad.extend(invariant_seeds);
    }
    assert!(bad.is_empty(), "{} of {n} seeds failed:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn fuzz_three_devices_new_seeds_converge_without_loss() {
    sweep(Faults::None, true, Pull::Whole);
}

#[test]
fn fuzz_with_network_faults_and_crashes_loses_no_line() {
    sweep(Faults::Network, false, Pull::Whole);
}

#[test]
fn fuzz_with_network_faults_and_crashes_converges() {
    sweep(Faults::Network, true, Pull::Whole);
}

/// Crashes and lost responses between the batches of a pull lose no line
/// either, and the devices converge.
#[test]
fn fuzz_in_small_batches_with_network_faults_and_crashes() {
    sweep(Faults::Network, true, Pull::SmallBatches);
}

/// Files that another device changes while a pull is between two batches
/// come twice in that pull; no line is lost and the devices converge.
#[test]
fn fuzz_in_small_batches_with_changes_during_the_pull() {
    sweep(Faults::None, true, Pull::SmallBatchesWithChanges);
}

/// A crash after a pulled rename and edit does not make the next sync take
/// a conflict copy for the renamed note and for another remote file at once,
/// with one overwriting the other.
#[test]
fn fuzz_seed_1356_with_faults_loses_no_line() {
    let o = run(1356, Faults::Network, 150, false, Pull::Whole);
    assert!(o.lost.is_empty(), "lines lost: {:?}", o.lost);
    assert!(o.invariant.is_empty(), "two file ids track one path: {:?}", o.invariant);
    assert!(o.diverged.is_none(), "{}", o.diverged.unwrap());
}

#[test]
fn fuzz_seed_1423_every_final_sync_succeeds() {
    let o = run(1423, Faults::None, 150, false, Pull::Whole);
    assert!(o.lost.is_empty(), "{:?}", o.lost);
    assert!(o.invariant.is_empty(), "two file ids track one path: {:?}", o.invariant);
    assert!(o.diverged.is_none(), "{}", o.diverged.unwrap());
}
