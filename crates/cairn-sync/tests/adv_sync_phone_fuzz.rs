//! Randomized sync of two PCs whose file systems ignore case (Windows,
//! macOS, as `CaseInsensitiveFs`), a phone on Android shared storage
//! (SafPlugin.kt, as `SharedStorageFs`) and, for every fourth seed on Linux,
//! a desktop whose file system tells case apart (`StdFs`), in folders that
//! they spell differently. Notes are created, edited, renamed, moved between
//! folders and deleted, outside the app or in it, and the devices sync in
//! random order. For every other seed without the desktop, the PCs and the
//! phone also rename folders by case only. The desktop makes its notes in
//! any spelling of a folder, so it has several folders whose names differ
//! only in case, and now and then a note with the name of one it has in
//! another spelling of the folder. For every third seed the phone pulls one
//! record per batch, as it pulls a large notebook. Now and then the phone's
//! renames of a folder fail in a sync, before anything is renamed or
//! halfway, as shared storage can fail them (`RenameFault`).
//!
//! Each note starts with the line it was made with, which names it (edits
//! never go before it). Invariants:
//! 1. every sync succeeds;
//! 2. syncing the devices in turn, they settle within a few passes: one in
//!    which no device uploads anything, or lists a file as not synced;
//! 3. then all have the same notes with the same text, by path ignoring
//!    case, and none has two files whose names differ only in case in one
//!    folder, nor two such folders, but the desktop, nor a folder under a
//!    free name that a rename of the phone left ("Drafts (1)");
//! 4. every line written is on every device, but those that were in a note
//!    when a device deleted it;
//! 5. a note that a device deleted is there again only if another device
//!    edited, renamed or moved it, or renamed its folder, before it had the
//!    delete (but for a note that another file may start with the first line
//!    of too: a conflict copy, or one renamed or moved outside the app);
//! 6. a conflict copy is there only for a note that two devices edited, each
//!    before it had the other's edit (a rename or move that brings a note
//!    back over a delete, as in 5, writes that device's version of it), or
//!    for a note that the desktop made
//!    in two spellings of a folder (but for one renamed or moved outside the
//!    app: after an edit that makes it two notes, FINDING-062);
//! 7. where no device renames folders, the server's folder of a note, as
//!    each device's sync state has it after each of its syncs, never
//!    changes, also in case, but by a rename or a move of the note, and the
//!    desktop keeps a note in the folder it first had it in, unless one was
//!    renamed or moved;
//! 8. where they do, the spelling that the phone last gave a folder is the
//!    server's, at the end, for the notes it had synced in it then, unless
//!    another device renamed that folder, or one in it or above it, before
//!    it had the phone's rename, or the note was moved to another folder
//!    since, or renamed or moved outside the app, or renamed or edited by a
//!    device that had not the phone's rename yet and deleted by another.
//!
//! Environment (all optional):
//!   CAIRN_PHONE_FUZZ_SEEDS   number of seeds (default 4)
//!   CAIRN_PHONE_FUZZ_START   first seed (default 0)
//!   CAIRN_PHONE_FUZZ_STEPS   steps per seed (default 80)
//!   CAIRN_PHONE_FUZZ_THREADS worker threads (default 2)
//!   CAIRN_PHONE_FUZZ_LOG=1   print the operation log of failing seeds (=all:
//!                            of every seed)
//!   CAIRN_PHONE_FUZZ_TRACE=t add to the log, after each step, where each
//!                            device has the note made with line t, and what
//!                            its sync state has of it
//!   CAIRN_PHONE_FUZZ_ENGINE_LOG=1 print the sync engine's log and each step
//!                            as they come (best with one seed and thread)
//!
//!   cargo test -p cairn-sync --test adv_sync_phone_fuzz
//!   CAIRN_PHONE_FUZZ_SEEDS=300 CAIRN_PHONE_FUZZ_THREADS=4 cargo test -p cairn-sync --test adv_sync_phone_fuzz -- --nocapture

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::sync::Arc;

use cairn_sync::engine::{SyncReport, Tracked};
use common::*;
use parking_lot::Mutex;

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

/// The folders of the notebook, each level with the spellings the devices
/// give it.
const FOLDERS: &[&[&[&str]]] = &[
    &[&["Drafts", "DRAFTS", "drafts"]],
    &[&["Inbox", "INBOX", "inbox"]],
    &[&["Work", "WORK", "work"], &["Notes", "NOTES", "notes"]],
];

/// A note no step touches: a notebook folder with no files at all does not
/// sync (FINDING-006), and deleting every note is not what this checks.
const ANCHOR: &str = "anchor.md";

const PHONE: usize = 2;
const LINUX: usize = 3;

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

fn parent(p: &str) -> &str {
    p.rfind('/').map_or("", |i| &p[..i])
}

fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn is_copy(p: &str) -> bool {
    file_name(p).contains(" (conflict ")
}

/// The name in folder `dir` on `d`'s disk that is `name` ignoring case, if
/// there is one.
fn on_disk(d: &Device, dir: &str, name: &str) -> Option<String> {
    let found = fs::read_dir(d.root.join(dir)).ok()?.flatten().map(|e| e.file_name().to_string_lossy().to_string());
    found.into_iter().find(|n| n.to_lowercase() == name.to_lowercase())
}

/// The first `levels` levels of folder `f` as `d` spells them on its disk.
/// A level that is not there yet gets one of its spellings from `rng`, as
/// the user makes it.
fn spelled(d: &Device, f: &[&[&str]], levels: usize, rng: &mut Rng) -> String {
    let mut path = String::new();
    for spellings in &f[..levels] {
        let name = on_disk(d, &path, spellings[0]).unwrap_or_else(|| rng.pick(spellings).to_string());
        path = join(&path, &name);
    }
    path
}

/// Folder `f` in any of its spellings, as the desktop that tells case apart
/// makes notes in it.
fn any_spelling(f: &[&[&str]], rng: &mut Rng) -> String {
    f.iter().fold(String::new(), |path, spellings| join(&path, rng.pick(spellings)))
}

/// The folder of `FOLDERS` that `dir` is, ignoring case.
fn folder_of(dir: &str) -> Option<&'static [&'static [&'static str]]> {
    FOLDERS.iter().copied().find(|f| f.iter().map(|s| s[0]).collect::<Vec<_>>().join("/").to_lowercase() == dir.to_lowercase())
}

/// Names on `d`'s disk that differ only in case from another one in the
/// same folder; with `files_only`, of files.
fn case_twins(d: &Device, files_only: bool) -> Vec<String> {
    fn walk(dir: &std::path::Path, rel: &str, files_only: bool, out: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        let mut seen: Vec<String> = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.path().is_dir();
            if !(files_only && is_dir) {
                if seen.iter().any(|s| s.to_lowercase() == name.to_lowercase()) {
                    out.push(join(rel, &name));
                }
                seen.push(name.clone());
            }
            if is_dir {
                walk(&e.path(), &join(rel, &name), files_only, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&d.root, "", files_only, &mut out);
    out
}

/// The notes on `d` (but the anchor), each with its first line.
fn notes(d: &Device) -> Vec<(String, String)> {
    d.files().into_iter().filter(|(p, _)| p != ANCHOR).map(|(p, c)| (p, c.lines().next().unwrap_or("").to_string())).collect()
}

/// The server's path of a file that a sync state tracks.
fn on_server(t: &Tracked) -> String {
    t.server_spelling.clone().or_else(|| t.server_path.clone()).unwrap_or_else(|| t.path.clone())
}

/// Where each device has the note made with line `token`, and what its
/// sync state has of it: the path on the server, `server_path`,
/// `server_spelling`, `renamed` and the seq.
fn trace(devs: &[Device], token: &str) -> String {
    let mut out = Vec::new();
    for d in devs {
        let state = d.engine.state();
        for (p, _) in notes(d).into_iter().filter(|(_, n)| n == token) {
            let tracked = state.files.values().find(|t| !t.deleted && t.path == p);
            let what = tracked.map(|t| format!("{} {:?} {:?} {} {}", on_server(t), t.server_path, t.server_spelling, t.renamed, t.seq));
            out.push(format!("{} {p} ({})", d.name, what.unwrap_or_else(|| "untracked".into())));
        }
    }
    out.join(", ")
}

/// What a step did to a note besides editing it, and whether it was in
/// the app. A note renamed or moved outside the app after an edit, also by
/// a rename of its folder there, goes up as a new note and the delete of
/// the old one (FINDING-062), in the spelling of the device that renamed
/// it: two files can start with its first line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Touch {
    /// Renamed in its folder.
    Renamed { in_app: bool },
    /// Moved to another folder.
    Moved { in_app: bool },
    /// Its folder was renamed by case.
    Folder { in_app: bool },
}

impl Touch {
    fn outside_the_app(self) -> bool {
        matches!(self, Touch::Renamed { in_app: false } | Touch::Moved { in_app: false } | Touch::Folder { in_app: false })
    }
}

/// A rename of a folder by case on the phone: the folder's name then, the
/// step, and the synced files in it, by file id, with their notes.
type PhoneRename = (String, u64, Vec<(String, String)>);

/// What the run did, for the invariants.
#[derive(Default)]
struct History {
    /// For each device, the steps at which it synced with nothing listed as
    /// not synced: then it had every change on the server, and the server
    /// had all of its own.
    clean: Vec<Vec<u64>>,
    /// For each note (by its first line), the edits made to it or to a
    /// conflict copy of it: (device, step).
    edits: HashMap<String, Vec<(usize, u64)>>,
    /// For each note, the renames and moves that took it along.
    moves: HashMap<String, Vec<(usize, u64, Touch)>>,
    /// For each note, its last delete.
    deletes: HashMap<String, (usize, u64)>,
    /// Renames of a folder by case: (device, step, the folder in lower case).
    folder_renames: Vec<(usize, u64, String)>,
    /// The notes that the desktop made with the name of a note of another
    /// spelling of their folder.
    twins: HashSet<String>,
    /// For each folder that the phone renamed by case (in lower case), its
    /// last such rename.
    phone_renames: HashMap<String, PhoneRename>,
    /// For each device, the server's path of each file in its sync state as
    /// last seen, with the file's note.
    seen: Vec<HashMap<String, (String, String)>>,
    /// The folder that the desktop first had each note in.
    linux_home: HashMap<String, String>,
    /// The notes that had a conflict copy on a device after one of its
    /// syncs: the copy starts with the note's first line too.
    copied: HashSet<String>,
    /// The folders (in lower case) that the phone renamed by case since it
    /// last synced. A note that comes into such a folder in that sync goes
    /// in as the phone spells it, and its rename goes up with the phone's
    /// (see `follow_spelling` in engine.rs): it is renamed by the phone too.
    renaming: Vec<String>,
}

impl History {
    /// The step of `d`'s first clean sync after `t` (`u64::MAX`: none).
    fn pushed(&self, d: usize, t: u64) -> u64 {
        self.clean[d].iter().copied().find(|&s| s > t).unwrap_or(u64::MAX)
    }

    /// The step of `d`'s last clean sync before `t` (0: none).
    fn pulled(&self, d: usize, t: u64) -> u64 {
        self.clean[d].iter().copied().take_while(|&s| s < t).last().unwrap_or(0)
    }

    /// Whether device `a` made a change at step `s` before it had the one
    /// that device `b` made at step `t`, and the other way round.
    fn concurrent(&self, (a, s): (usize, u64), (b, t): (usize, u64)) -> bool {
        a != b && self.pushed(a, s) > self.pulled(b, t) && self.pushed(b, t) > self.pulled(a, s)
    }

    /// Note a successful sync of device `i` at step `at`, and check
    /// invariant 7 against what its sync state and its disk show now
    /// (`spelling`: no device renames folders in this run).
    fn synced(&mut self, devs: &[Device], i: usize, at: u64, r: &SyncReport, spelling: bool) -> Result<(), String> {
        if r.skipped.is_empty() {
            self.clean[i].push(at);
        }
        let d = &devs[i];
        self.copied.extend(notes(d).into_iter().filter(|(p, _)| is_copy(p)).map(|(_, n)| n));
        if i == PHONE {
            for folder in std::mem::take(&mut self.renaming) {
                for (_, note) in notes(d).into_iter().filter(|(p, _)| p.to_lowercase().starts_with(&format!("{folder}/"))) {
                    self.moves.entry(note).or_default().push((i, at - 1, Touch::Folder { in_app: true }));
                }
            }
        }
        let mut seen = HashMap::new();
        for (fid, t) in d.engine.state().files.iter().filter(|(_, t)| !t.deleted) {
            let server = on_server(t);
            let before = self.seen[i].get(fid);
            let note = d.read(&t.path).and_then(|c| c.lines().next().map(str::to_string)).or_else(|| before.map(|b| b.1.clone())).unwrap_or_default();
            if spelling
                && let Some((was, _)) = before
                && parent(was) != parent(&server)
                && !self.moves.contains_key(&note)
            {
                return Err(format!("{}: {note} went from {was} to {server} on the server, though no one renamed or moved it", d.name));
            }
            seen.insert(fid.clone(), (server, note));
        }
        self.seen[i] = seen;
        if spelling && i == LINUX {
            for (p, note) in notes(d).into_iter().filter(|(p, _)| !is_copy(p)) {
                let home = self.linux_home.entry(note.clone()).or_insert_with(|| parent(&p).to_string());
                if home != parent(&p) && !self.moves.contains_key(&note) {
                    return Err(format!("linux: {note} went from {home} to {p}, though no one renamed or moved it"));
                }
            }
        }
        Ok(())
    }
}

/// Add `line` to the operation log, and print it at once with
/// CAIRN_PHONE_FUZZ_ENGINE_LOG.
fn push_log(log: &mut Vec<String>, line: String) {
    if std::env::var("CAIRN_PHONE_FUZZ_ENGINE_LOG").is_ok() {
        eprintln!("step {line}");
    }
    log.push(line);
}

/// Prints the sync engine's log as it comes (CAIRN_PHONE_FUZZ_ENGINE_LOG).
struct EngineLog;

impl log::Log for EngineLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("cairn_sync")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("engine {:?}: {}", std::thread::current().id(), r.args());
        }
    }
    fn flush(&self) {}
}

fn run(seed: u64, steps: u64) -> Result<(), String> {
    if std::env::var("CAIRN_PHONE_FUZZ_ENGINE_LOG").is_ok() && log::set_logger(&EngineLog).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let srv = server();
    let probe = Arc::new(StorageProbe::default());
    let storage = probe.clone();
    let linux = cfg!(target_os = "linux") && seed % 4 == 3;
    let renames = !linux && seed.is_multiple_of(2);
    let mut devs = vec![
        Device::with_fs(&srv, "win-a", &[(ANCHOR, "untouched\n")], CaseInsensitiveFs::new),
        Device::with_fs(&srv, "win-b", &[], CaseInsensitiveFs::new),
        Device::with_fs(&srv, "phone", &[], move |root| SharedStorageFs::probed(root, storage)),
    ];
    if linux {
        devs.push(Device::new(&srv, "linux", &[]));
    }
    if seed % 3 == 1 {
        devs[PHONE].pull_one_by_one();
    }
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5AFE_F01D);
    let mut log: Vec<String> = Vec::new();
    let fail = |log: &[String], e: String| -> String {
        let ops = if std::env::var("CAIRN_PHONE_FUZZ_LOG").is_ok() { log.join("\n") } else { format!("({} steps; set CAIRN_PHONE_FUZZ_LOG=1 for the operation log)", log.len()) };
        format!("seed {seed}: {e}\n{ops}")
    };
    let mut h = History { clean: vec![Vec::new(); devs.len()], seen: vec![HashMap::new(); devs.len()], ..History::default() };
    // Lines written, and those that were in a note when it was deleted.
    let mut written: Vec<String> = Vec::new();
    let mut gone: HashSet<String> = HashSet::new();
    let mut counter = 0u64;
    let mut clock = 0u64;
    for i in 0..devs.len() {
        clock += 1;
        match devs[i].try_sync() {
            Ok(r) => h.synced(&devs, i, clock, &r, !renames).map_err(|e| fail(&log, e))?,
            Err(e) => return Err(fail(&log, format!("the first sync on {} failed: {e}", devs[i].name))),
        }
    }
    for step in 0..steps {
        clock += 1;
        let i = rng.below(devs.len() as u64) as usize;
        let d = &devs[i];
        let op = rng.below(100);
        let notes_here: Vec<(String, String)> = notes(d);
        let in_app = rng.chance(50);
        let how = if in_app { " in the app" } else { "" };
        let mut synced = None;
        let desc = match op {
            // A new note, in a folder as this device spells it; on the
            // desktop that tells case apart, in any spelling, and now and
            // then with the name of a note it has in another spelling of
            // that folder.
            0..=21 => {
                counter += 1;
                let line = format!("tok{counter}");
                let mut twin = None;
                let others: Vec<&(String, String)> = notes_here.iter().filter(|(p, _)| !is_copy(p) && folder_of(parent(p)).is_some()).collect();
                if i == LINUX && !others.is_empty() && rng.chance(30) {
                    let (p, note) = *rng.pick(&others);
                    let f = folder_of(parent(p)).unwrap();
                    let other = (0..6).map(|_| any_spelling(f, &mut rng)).find(|s| s != parent(p)).map(|o| join(&o, file_name(p)));
                    twin = other.filter(|t| !d.exists(t)).map(|t| (t, note.clone()));
                }
                let path = match twin {
                    Some((path, note)) => {
                        h.twins.insert(note);
                        h.twins.insert(line.clone());
                        path
                    }
                    None => {
                        let f = *rng.pick(FOLDERS);
                        let dir = if i == LINUX { any_spelling(f, &mut rng) } else { spelled(d, f, f.len(), &mut rng) };
                        join(&dir, &format!("n{counter}.md"))
                    }
                };
                d.write(&path, &format!("{line}\n"));
                written.push(line);
                format!("new {path}")
            }
            // An edit: a line anywhere in a note but before its first.
            22..=39 if !notes_here.is_empty() => {
                let (path, note) = rng.pick(&notes_here).clone();
                counter += 1;
                let line = format!("tok{counter}");
                let old = d.read(&path).unwrap_or_default();
                let mut lines: Vec<&str> = old.lines().collect();
                let at = 1 + rng.below(lines.len().max(1) as u64) as usize;
                lines.insert(at.min(lines.len()), &line);
                d.write(&path, &(lines.join("\n") + "\n"));
                written.push(line);
                h.edits.entry(note).or_default().push((i, clock));
                format!("edit {path} at line {at}")
            }
            // A move into another folder.
            40..=46 if !notes_here.is_empty() => {
                let (path, note) = rng.pick(&notes_here).clone();
                let f = *rng.pick(FOLDERS);
                let dir = if i == LINUX { any_spelling(f, &mut rng) } else { spelled(d, f, f.len(), &mut rng) };
                let to = join(&dir, file_name(&path));
                if parent(&to).to_lowercase() == parent(&path).to_lowercase() || d.paths().iter().any(|p| p.to_lowercase() == to.to_lowercase()) {
                    format!("skip move {path} -> {to}")
                } else {
                    if in_app {
                        d.app_mv(&path, &to);
                    } else {
                        d.mv(&path, &to);
                    }
                    h.moves.entry(note).or_default().push((i, clock, Touch::Moved { in_app }));
                    format!("move {path} -> {to}{how}")
                }
            }
            // A note renamed in its folder.
            47..=51 if !notes_here.is_empty() => {
                let (path, note) = rng.pick(&notes_here).clone();
                counter += 1;
                let to = join(parent(&path), &format!("r{counter}.md"));
                if in_app {
                    d.app_mv(&path, &to);
                } else {
                    d.mv(&path, &to);
                }
                h.moves.entry(note).or_default().push((i, clock, Touch::Renamed { in_app }));
                format!("rename {path} -> {to}{how}")
            }
            // A folder renamed by case only.
            52..=58 if renames => {
                let f = *rng.pick(FOLDERS);
                let level = rng.below(f.len() as u64) as usize;
                let above = spelled(d, f, level, &mut rng);
                match on_disk(d, &above, f[level][0]) {
                    Some(name) if d.root.join(&above).join(&name).is_dir() => {
                        let to = rng.pick(f[level]).to_string();
                        let (from, to) = (join(&above, &name), join(&above, &to));
                        if from == to {
                            format!("skip folder rename {from}")
                        } else {
                            let inside: Vec<String> = notes_here.iter().filter(|(p, _)| p.starts_with(&format!("{from}/"))).map(|(_, n)| n.clone()).collect();
                            if in_app {
                                d.app_mv(&from, &to);
                            } else {
                                d.mv(&from, &to);
                            }
                            for note in &inside {
                                h.moves.entry(note.clone()).or_default().push((i, clock, Touch::Folder { in_app }));
                            }
                            h.folder_renames.push((i, clock, from.to_lowercase()));
                            if i == PHONE {
                                h.renaming.push(from.to_lowercase());
                                // The files it had synced: one made since goes up as the phone spells its folder when it syncs.
                                let synced: HashMap<&str, &String> = d.engine.state().files.iter().filter(|(_, t)| !t.deleted).map(|(f, t)| (t.path.as_str(), f)).collect();
                                let had = notes_here
                                    .iter()
                                    .filter(|(_, n)| inside.contains(n))
                                    .filter_map(|(p, n)| Some(((*synced.get(p.as_str())?).clone(), n.clone())))
                                    .collect();
                                h.phone_renames.insert(from.to_lowercase(), (file_name(&to).to_string(), clock, had));
                            }
                            format!("folder {from} -> {to}{how}")
                        }
                    }
                    _ => "skip folder rename (none there)".to_string(),
                }
            }
            // A note deleted, with the lines in it.
            59..=62 if !notes_here.is_empty() => {
                let (path, note) = rng.pick(&notes_here).clone();
                gone.extend(d.read(&path).unwrap_or_default().lines().map(str::to_string));
                d.rm(&path);
                if !is_copy(&path) {
                    h.deletes.insert(note, (i, clock));
                }
                format!("delete {path}")
            }
            // The phone syncs, and its renames of one folder, in any of its
            // spellings, fail in this sync.
            63..=70 if i == PHONE => {
                let f = *rng.pick(FOLDERS);
                let level = rng.below(f.len() as u64) as usize;
                let fault = if rng.chance(50) { RenameFault::Fail } else { RenameFault::StopHalfway };
                let mut targets = vec![String::new()];
                for spellings in &f[..=level] {
                    targets = targets.iter().flat_map(|t| spellings.iter().map(move |s| join(t, s))).collect();
                }
                for t in &targets {
                    probe.add(t, fault);
                }
                let r = devs[i].try_sync();
                probe.clear();
                match r {
                    Ok(r) => {
                        let desc = format!(
                            "sync, renames to {targets:?} failing ({fault:?}): pulled {} pushed {} conflicts {:?} not synced {:?}",
                            r.pulled, r.pushed, r.conflicts, r.skipped
                        );
                        synced = Some(r);
                        desc
                    }
                    Err(e) => return Err(fail(&log, format!("sync on {} failed at step {step}: {e}", devs[i].name))),
                }
            }
            _ => match devs[i].try_sync() {
                Ok(r) => {
                    let desc = format!("sync: pulled {} pushed {} conflicts {:?} not synced {:?}", r.pulled, r.pushed, r.conflicts, r.skipped);
                    synced = Some(r);
                    desc
                }
                Err(e) => return Err(fail(&log, format!("sync on {} failed at step {step}: {e}", devs[i].name))),
            },
        };
        push_log(&mut log, format!("{step}: {} {desc}", devs[i].name));
        if let Ok(t) = std::env::var("CAIRN_PHONE_FUZZ_TRACE") {
            push_log(&mut log, format!("   {t}: {}", trace(&devs, &t)));
        }
        if let Some(r) = synced {
            h.synced(&devs, i, clock, &r, !renames).map_err(|e| fail(&log, e))?;
        }
    }
    // A change that waits as pending, listed, is tried again in the next
    // sync, so a pass with no upload may still leave one to come.
    let mut listed: Vec<String> = Vec::new();
    let mut settled = false;
    for pass in 0..8 {
        let mut pushed = 0;
        listed.clear();
        for i in 0..devs.len() {
            clock += 1;
            let r = match devs[i].try_sync() {
                Ok(r) => r,
                Err(e) => return Err(fail(&log, format!("sync on {} failed in settling pass {pass}: {e}", devs[i].name))),
            };
            push_log(&mut log, format!("settle {pass}: {} pulled {} pushed {} conflicts {:?}", devs[i].name, r.pulled, r.pushed, r.conflicts));
            if let Ok(t) = std::env::var("CAIRN_PHONE_FUZZ_TRACE") {
                push_log(&mut log, format!("   {t}: {}", trace(&devs, &t)));
            }
            h.synced(&devs, i, clock, &r, !renames).map_err(|e| fail(&log, e))?;
            pushed += r.pushed;
            listed.extend(r.skipped.iter().map(|s| format!("{}: {} ({})", devs[i].name, s.path, s.reason)));
        }
        if pushed == 0 && listed.is_empty() {
            settled = true;
            break;
        }
    }
    if !settled && !listed.is_empty() {
        return Err(fail(&log, format!("files still not synced after 8 passes: {listed:?}")));
    }
    if !settled {
        return Err(fail(&log, "the devices kept uploading after 8 passes".into()));
    }
    let by_path = |d: &Device| -> BTreeMap<String, String> { d.files().into_iter().map(|(p, c)| (p.to_lowercase(), c)).collect() };
    let first = by_path(&devs[0]);
    for d in &devs[1..] {
        if by_path(d) != first {
            return Err(fail(&log, format!("{} differs from {}: {}", d.name, devs[0].name, tree_diff(&devs[0].name, &devs[0].files_bytes(), &d.name, &d.files_bytes()))));
        }
    }
    for (i, d) in devs.iter().enumerate() {
        let twins = case_twins(d, i == LINUX);
        if !twins.is_empty() {
            return Err(fail(&log, format!("{} has names that differ only in case in one folder: {twins:?}", d.name)));
        }
        // Shared storage renames a folder by case through a free name, which
        // a sync that stops halfway leaves (see `RenameFault::StopHalfway`).
        let free: Vec<String> = d.paths().iter().map(|p| parent(p).to_string()).filter(|f| f.split('/').any(|n| n.ends_with(')') && n.contains(" ("))).collect();
        if !free.is_empty() {
            return Err(fail(&log, format!("{} has folders under a free name: {free:?}", d.name)));
        }
        let lines: HashSet<String> = d.files().iter().flat_map(|(_, c)| c.lines().map(str::to_string).collect::<Vec<_>>()).collect();
        let lost: Vec<&String> = written.iter().filter(|l| !gone.contains(*l) && !lines.contains(*l)).collect();
        if !lost.is_empty() {
            return Err(fail(&log, format!("lines lost on {}: {lost:?}", d.name)));
        }
    }
    let end = notes(&devs[0]);
    let there: HashSet<&str> = end.iter().filter(|(p, _)| !is_copy(p)).map(|(_, n)| n.as_str()).collect();
    // 5 and 6 are not for a note whose first line another file may have
    // too: a conflict copy, or the new note that a rename or move outside
    // the app makes of an edited one (FINDING-062).
    let unsure = |note: &String| h.copied.contains(note) || h.moves.get(note).into_iter().flatten().any(|&(_, _, m)| m.outside_the_app());
    // 5: deletes.
    for (note, &(by, at)) in &h.deletes {
        if !there.contains(note.as_str()) || unsure(note) {
            continue;
        }
        let mut touched = h.edits.get(note).into_iter().flatten().copied().chain(h.moves.get(note).into_iter().flatten().map(|&(d, t, _)| (d, t)));
        if !touched.any(|e| h.concurrent((by, at), e)) {
            return Err(fail(&log, format!("{note}, deleted on {} at {at}, is back, though no other device changed it meanwhile", devs[by].name)));
        }
    }
    // 6: conflict copies. A rename or move that brought a note back over a
    // delete made meanwhile (see 5) writes that device's version of it
    // again, as it had it, over the edits it had not had yet: a write too.
    for (path, note) in end.iter().filter(|(p, _)| is_copy(p)) {
        let mut edits = h.edits.get(note).cloned().unwrap_or_default();
        if let Some(&delete) = h.deletes.get(note) {
            let touched = h.moves.get(note).into_iter().flatten().map(|&(d, t, _)| (d, t));
            edits.extend(touched.filter(|&m| h.concurrent(delete, m)));
        }
        let concurrent = edits.iter().any(|a| edits.iter().any(|b| h.concurrent(*a, *b)));
        let outside = h.moves.get(note).into_iter().flatten().any(|&(_, _, m)| m.outside_the_app());
        if !concurrent && !h.twins.contains(note) && !outside {
            return Err(fail(&log, format!("{path} is a conflict copy of {note}, which no two devices edited at once")));
        }
    }
    // 8: the phone's own renames of a folder, for the files it had synced
    // in it, by file id: another file that starts with the note's line (a
    // conflict copy renamed, FINDING-062) goes up as its own device spells
    // the folder.
    let server: HashMap<String, String> =
        devs[0].engine.state().files.iter().filter(|(_, t)| !t.deleted).map(|(f, t)| (f.clone(), on_server(t))).collect();
    for (folder, (name, at, inside)) in &h.phone_renames {
        let since = h.pulled(PHONE, *at);
        // Another device that renames a folder in it, or above it, uploads
        // its own spelling of the folders above that too.
        let related = |f: &str| f == folder || f.starts_with(&format!("{folder}/")) || folder.starts_with(&format!("{f}/"));
        if h.folder_renames.iter().any(|(d, t, f)| *d != PHONE && related(f) && h.pushed(*d, *t) > since) {
            continue;
        }
        let level = folder.matches('/').count();
        for (fid, note) in inside {
            // A rename or an edit made by a device that had not the phone's
            // rename yet, of a note that a device deleted, wins over the
            // delete with that device's path, as it had it.
            let deleted = h.deletes.contains_key(note);
            let theirs = |(d, t): (usize, u64)| deleted && d != PHONE && h.concurrent((PHONE, *at), (d, t));
            let moved = h.moves.get(note).into_iter().flatten().any(|&(d, t, m)| {
                (matches!(m, Touch::Moved { .. }) && h.pushed(d, t) > since)
                    || (m.outside_the_app() && h.pushed(d, t) > since)
                    || (matches!(m, Touch::Renamed { .. }) && theirs((d, t)))
            }) || h.edits.get(note).into_iter().flatten().any(|&e| theirs(e));
            let inside = |p: &&String| {
                let dir = parent(p).to_lowercase();
                dir == *folder || dir.starts_with(&format!("{folder}/"))
            };
            let Some(path) = server.get(fid).filter(|p| !moved && inside(p)) else { continue };
            if path.split('/').nth(level) != Some(name.as_str()) {
                return Err(fail(&log, format!("the phone renamed {folder} to {name} at {at}, but {note} is at {path} on the server")));
            }
        }
    }
    if std::env::var("CAIRN_PHONE_FUZZ_LOG").as_deref() == Ok("all") {
        eprintln!("seed {seed} ok:\n{}\nphone: {:?}\n", log.join("\n"), devs[PHONE].paths());
    }
    Ok(())
}

#[test]
fn randomized_two_pcs_and_a_phone_on_shared_storage_that_spell_folders_differently() {
    let seeds = env_u64("CAIRN_PHONE_FUZZ_SEEDS", 4);
    let start = env_u64("CAIRN_PHONE_FUZZ_START", 0);
    let steps = env_u64("CAIRN_PHONE_FUZZ_STEPS", 80);
    let threads = env_u64("CAIRN_PHONE_FUZZ_THREADS", 2).max(1);
    let next = Arc::new(std::sync::atomic::AtomicU64::new(start));
    let failures: Arc<Mutex<Vec<(u64, String)>>> = Arc::new(Mutex::new(Vec::new()));
    std::thread::scope(|s| {
        for _ in 0..threads {
            let (next, failures) = (next.clone(), failures.clone());
            s.spawn(move || {
                loop {
                    let seed = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if seed >= start + seeds {
                        break;
                    }
                    let r = std::panic::catch_unwind(|| run(seed, steps)).unwrap_or_else(|p| {
                        let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                        Err(format!("seed {seed}: panic: {}", msg.unwrap_or_default()))
                    });
                    if let Err(e) = r {
                        eprintln!("FAIL {e}\n");
                        failures.lock().push((seed, e));
                    }
                }
            });
        }
    });
    let mut failed = failures.lock().clone();
    failed.sort_by_key(|f| f.0);
    let ids: Vec<u64> = failed.iter().map(|f| f.0).collect();
    eprintln!("{} of {seeds} seeds failed: {ids:?}", failed.len());
    assert!(failed.is_empty(), "{} of {seeds} seeds failed: {ids:?}\nfirst: {}", failed.len(), failed[0].1);
}
