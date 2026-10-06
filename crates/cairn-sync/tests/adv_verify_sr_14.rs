//! Reproduction for FINDING-145 (two live file ids tracking one
//! path in state.json, followed by a failed sync).
//!
//! `replay_seed_1423_with_head_log` replays adv_sync_robust_fuzz.rs seed 1423
//! with the same RNG consumption, but with a transport that decrypts and logs
//! every pulled head and every pushed revision of the tablet, plus a dump of
//! the tablet's state for the affected paths before and after each sync.
//! Set SR14_LOG=1 to print the log.
//!
//! `conflict_name_reuses_path_of_locally_renamed_file` is a minimal,
//! deterministic two-device reproduction of the mechanism found with the log.
//!
//! cargo test -p cairn-sync --test adv_verify_sr_14

#[path = "adv_sync_robust_common.rs"]
mod common;

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cairn_sync::crypto::{FilePayload, VaultKey};
use cairn_sync::protocol::*;
use cairn_sync::transport::{PutOutcome, Transport};
use cairn_sync::SyncError;
use common::*;

struct LogTransport {
    inner: Box<dyn Transport>,
    key: [u8; 32],
    name: String,
    on: Arc<AtomicBool>,
}

impl LogTransport {
    fn path(&self, fid: &str, blob: &str) -> String {
        let k = VaultKey::from_bytes(self.key);
        unb64(blob)
            .and_then(|b| k.decrypt(fid, &b).ok())
            .and_then(|p| FilePayload::decode(&p).ok())
            .map(|p| p.path)
            .unwrap_or_else(|| "?".into())
    }
}

impl Transport for LogTransport {
    fn get_vault(&self, v: &str) -> Result<Option<VaultInfo>, SyncError> {
        self.inner.get_vault(v)
    }
    fn create_vault(&self, v: &str, k: &KeyEnvelope) -> Result<(), SyncError> {
        self.inner.create_vault(v, k)
    }
    fn changes(&self, v: &str, s: u64, l: u32) -> Result<ChangesResponse, SyncError> {
        let r = self.inner.changes(v, s, l)?;
        if self.on.load(Ordering::SeqCst) {
            for h in &r.heads {
                let p = self.path(&h.file_id, &h.blob);
                if p.contains("att/a1") || p.contains("r1/m2") || p.contains("r1/m6") {
                    eprintln!("      {} pull head {} seq {} del {} path {p}", self.name, &h.file_id[..6], h.seq, h.deleted);
                }
            }
        }
        Ok(r)
    }
    fn put(&self, v: &str, f: &str, r: &PutRevision) -> Result<PutOutcome, SyncError> {
        let out = self.inner.put(v, f, r)?;
        if self.on.load(Ordering::SeqCst) {
            let p = self.path(f, &r.blob);
            let res = match &out {
                PutOutcome::Stored(s) => format!("stored seq {s}"),
                PutOutcome::Conflict(c) => format!("conflict {c:?}"),
            };
            eprintln!("      {} push {} parent {:?} del {} path {p} -> {res}", self.name, &f[..6], r.parent_seq, r.deleted);
        }
        Ok(out)
    }
    fn history(&self, v: &str, f: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        self.inner.history(v, f)
    }
    fn revision(&self, v: &str, s: u64) -> Result<RevisionBlob, SyncError> {
        self.inner.revision(v, s)
    }
}

fn install_log(d: &mut Device, on: Arc<AtomicBool>) {
    let raw = fs::read_to_string(d.state_dir.join("key")).unwrap();
    let key: [u8; 32] = unb64(raw.trim()).unwrap().try_into().unwrap();
    let url = d.url.clone();
    let name = d.name.clone();
    d.set_transport(Box::new(LogTransport { inner: http(&url), key, name, on }));
}

fn dump_state(d: &Device, label: &str) {
    if let Some(e) = d.engine.as_ref() {
        for (fid, t) in &e.state().files {
            if !t.deleted && (t.path.contains("att/a1") || t.path.contains("r1/m2") || t.path.contains("r1/m6")) {
                eprintln!("      {} state {label}: {} seq {} -> {}", d.name, &fid[..6], t.seq, t.path);
            }
        }
    }
}

fn dup_paths(d: &Device) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(e) = d.engine.as_ref() {
        let mut seen = std::collections::HashMap::new();
        for (fid, t) in &e.state().files {
            if !t.deleted {
                if let Some(other) = seen.insert(t.path.clone(), fid.clone()) {
                    out.push(format!("{}: {other} and {fid} both track {}", d.name, t.path));
                }
            }
        }
    }
    out
}

/// Same operation sequence as adv_sync_robust_fuzz.rs `run(seed, Faults::None, steps)`.
#[test]
fn replay_seed_1423_with_head_log() {
    let log = std::env::var("SR14_LOG").is_ok();
    let on = Arc::new(AtomicBool::new(log));
    let srv = server();
    let mut devs = vec![Device::new(&srv, "laptop", &[]), Device::new(&srv, "phone", &[]), Device::new(&srv, "tablet", &[])];
    for d in devs.iter_mut() {
        install_log(d, on.clone());
    }
    let mut rng = Rng(1423u64.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut broken = Vec::new();
    for step in 0..150u64 {
        let d = rng.below(devs.len() as u64) as usize;
        let files: Vec<String> = devs[d].paths();
        let pick = |rng: &mut Rng| files[rng.below(files.len() as u64) as usize].clone();
        let op = rng.below(14);
        match op {
            0..=2 => {
                let p = format!("f{}/n{}.md", rng.below(3), rng.below(12));
                if !devs[d].root.join(&p).exists() {
                    devs[d].write(&p, &format!("created {step} on {d}\n"));
                }
            }
            3..=5 if !files.is_empty() => {
                let p = pick(&mut rng);
                let old = devs[d].read(&p).unwrap_or_default();
                devs[d].write(&p, &format!("{old}edit {step} on {d}\n"));
            }
            6 if !files.is_empty() => {
                let p = pick(&mut rng);
                if log && p.contains("att/a1") {
                    eprintln!("  step {step} {}: rm {p}", devs[d].name);
                }
                devs[d].rm(&p);
            }
            7 if !files.is_empty() => {
                let p = pick(&mut rng);
                let ext = if p.ends_with(".bin") { "bin" } else { "md" };
                let to = format!("r{}/m{}.{ext}", rng.below(2), rng.below(20));
                if !devs[d].root.join(&to).exists() {
                    if log && p.contains("att/a1") {
                        eprintln!("  step {step} {}: mv {p} -> {to}", devs[d].name);
                    }
                    devs[d].mv(&p, &to);
                }
            }
            8 if !files.is_empty() => {
                let p = pick(&mut rng);
                let ext = if p.ends_with(".bin") { "bin" } else { "md" };
                let to = format!("r{}/m{}.{ext}", rng.below(2), rng.below(20));
                if !devs[d].root.join(&to).exists() {
                    let old = devs[d].read(&p).unwrap_or_default();
                    if log && p.contains("att/a1") {
                        eprintln!("  step {step} {}: mv+edit {p} -> {to}", devs[d].name);
                    }
                    devs[d].mv(&p, &to);
                    devs[d].write(&to, &format!("{old}renedit {step} on {d}\n"));
                }
            }
            9 => {
                let p = format!("att/a{}.bin", rng.below(3));
                if log && p.contains("att/a1") {
                    eprintln!("  step {step} {}: append {p}", devs[d].name);
                }
                let old = devs[d].read(&p).unwrap_or_default();
                devs[d].write(&p, &format!("{old}bin {step} on {d}\n"));
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
                if log {
                    eprintln!("  step {step} sync {}", devs[d].name);
                    dump_state(&devs[d], "before");
                }
                let res = devs[d].sync();
                if log {
                    dump_state(&devs[d], "after");
                    eprintln!("    -> {:?}", res.as_ref().map(|r| (r.pulled, r.pushed, r.conflicts.clone())).map_err(|e| e.to_string()));
                }
                for b in dup_paths(&devs[d]) {
                    if log {
                        eprintln!("    INVARIANT BROKEN after step {step}: {b}");
                    }
                    broken.push(format!("after step {step}: {b}"));
                }
            }
        }
    }
    let mut final_errors = Vec::new();
    for _ in 0..3 {
        for d in devs.iter_mut() {
            if log {
                eprintln!("  final sync {}", d.name);
                dump_state(d, "before");
            }
            let r = d.sync();
            if log {
                eprintln!("    -> {:?}", r.as_ref().map(|r| (r.pulled, r.pushed)).map_err(|e| e.to_string()));
            }
            if let Err(e) = r {
                final_errors.push(format!("{}: {e}", d.name));
            }
        }
    }
    assert!(broken.is_empty() && final_errors.is_empty(), "invariant: {broken:?}; failed final syncs: {final_errors:?}");
}

fn wait_for_fresh_minute() {
    // conflict copy names carry the minute; keep the whole scenario in one
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    if secs % 60 >= 50 {
        std::thread::sleep(std::time::Duration::from_secs(61 - secs % 60));
    }
}

fn state_of(d: &Device) -> Vec<(String, String, u64)> {
    let mut v: Vec<_> = d
        .engine
        .as_ref()
        .unwrap()
        .state()
        .files
        .iter()
        .filter(|(_, t)| !t.deleted)
        .map(|(f, t)| (t.path.clone(), f[..6].to_string(), t.seq))
        .collect();
    v.sort();
    v
}

/// Minimal, deterministic reproduction of the mechanism seen in the seed-1423
/// log (step 139 on the tablet):
///
/// 1. Laptop and tablet both create `att/a.bin`. The tablet's first sync gives
///    the laptop's file (fid A1) the conflict name
///    Q = `att/a (conflict <minute> tablet).bin` and tracks A1 at Q.
/// 2. On the tablet (offline) the user moves Q to `r/moved.bin` and creates a
///    new `att/a.bin`.
/// 3. The laptop creates a new `att/a.bin` (fid B) and syncs.
/// 4. The tablet syncs. B is new to the tablet and its path is taken locally,
///    so `conflict_path("att/a.bin")` runs; Q is free on disk (step 2)
///    so B is written to Q and tracked at Q, while A1 is still tracked at Q.
///    With the defect, the push's classify() attributed the file at Q to A1
///    (Modified: A1's content was replaced by B's on the server) and B was
///    force-pushed at Q too; `r/moved.bin` was uploaded as a brand-new file.
///
/// Now the push's scan gives a path that two tracked files have
/// to the one whose content is there (B), so A1 is found renamed to
/// `r/moved.bin` and that rename is pushed.
#[test]
fn conflict_name_reuses_path_of_locally_renamed_file() {
    wait_for_fresh_minute();
    let srv = server();
    let mut laptop = Device::new(&srv, "laptop", &[("att/a.bin", "laptop original\n")]);
    laptop.sync_ok();
    let mut tablet = Device::new(&srv, "tablet", &[("att/a.bin", "tablet original\n")]);
    let r = tablet.sync_ok();
    assert_eq!(r.conflicts.len(), 1, "setup: expected one conflict copy, got {:?}", r.conflicts);
    let q = r.conflicts[0].clone();
    assert_eq!(tablet.read(&q).as_deref(), Some("laptop original\n"));
    laptop.sync_ok();
    // settle; make sure att/a.bin is free everywhere (the laptop may already
    // have given the tablet's file a conflict name of its own)
    tablet.sync_ok();
    if laptop.root.join("att/a.bin").exists() {
        laptop.mv("att/a.bin", "other.bin");
        laptop.sync_ok();
        tablet.sync_ok();
    }
    laptop.sync_ok();
    assert!(!tablet.root.join("att/a.bin").exists() && !laptop.root.join("att/a.bin").exists());
    assert_eq!(tablet.read(&q).as_deref(), Some("laptop original\n"));

    // 2. tablet (offline): the user moves the conflict copy away and makes a
    //    new att/a.bin
    tablet.mv(&q, "r/moved.bin");
    tablet.write("att/a.bin", "tablet second file\n");
    // 3. laptop: a new att/a.bin (a new file id B)
    laptop.write("att/a.bin", "laptop second file\n");
    laptop.sync_ok();

    // 4.
    let before = state_of(&tablet);
    let r1 = tablet.sync();
    let after = state_of(&tablet);
    let dups = dup_paths(&tablet);
    // let the others react, then sync the tablet twice more
    let rl = laptop.sync();
    let r2 = tablet.sync();
    let rl2 = laptop.sync();
    let r3 = tablet.sync();
    let res = |r: &Result<cairn_sync::engine::SyncReport, SyncError>| r.as_ref().map(|r| (r.pulled, r.pushed, r.conflicts.clone())).map_err(|e| e.to_string());
    eprintln!("tablet state before: {before:?}\ntablet state after:  {after:?}");
    eprintln!("tablet sync 1 {:?}\nlaptop {:?}\ntablet sync 2 {:?}\nlaptop {:?}\ntablet sync 3 {:?}", res(&r1), res(&rl), res(&r2), res(&rl2), res(&r3));
    eprintln!("tablet files {:?}\nlaptop files {:?}", tablet.files(), laptop.files());
    assert!(
        dups.is_empty() && r1.is_ok() && r2.is_ok() && r3.is_ok() && rl.is_ok() && rl2.is_ok(),
        "after the tablet's sync: {dups:?} (tablet state {after:?})"
    );
}
