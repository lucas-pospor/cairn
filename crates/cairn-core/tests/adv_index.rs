// Incremental index vs full rebuild.
//
// Random operations (in-app create/write/rename/move/delete, folder renames,
// external edits/creates/deletes/renames picked up by rescan or
// rescan_paths) are applied to one Vault; after every step its answers
// (entries, outgoing, backlinks, note_info, tags, graph, search, resolve)
// are compared with a fresh Vault::open of the same folder.
//
//   cargo test -p cairn-core --test adv_index
//   ADV_INDEX_SEEDS=50 ADV_INDEX_STEPS=300 cargo test -p cairn-core --test adv_index -- --nocapture
//   cargo test -p cairn-core --test adv_index finding_      (FINDING tests only)

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use cairn_core::{EntryKind, StdFs, TrashMode, Vault};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pct(&mut self, p: usize) -> bool {
        self.below(100) < p
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

const FOLDERS: &[&str] = &["", "", "a", "a/b", "notes", "Notes", "my folder", "ééé", "日本"];
/// False on a file system that ignores case (Windows, macOS): there
/// "Notes" is the folder "notes", and "Note.md" the file "note.md".
const CASE_SENSITIVE: bool = !cfg!(any(windows, target_os = "macos"));
const NAMES: &[&str] = &["Note", "note", "Alpha", "Beta", "Gamma", "Café", "Daily 2024", "x.y", "Leaf", "Ünï"];
const WORDS: &[&str] = &[
    "garden", "gardening", "river", "stone", "harvest", "programming", "program", "café", "naïve", "straße", "日本語",
    "emoji😀", "C++", "rust", "Rust", "the", "a", "zebra", "über",
];
const QUERIES: &[&str] = &[
    "garden", "gard", "river stone", "\"river stone\"", "tag:plants", "#plants/herbs", "path:notes", "harvest tag:home",
    "café", "日本語", "note", "alpha", "prog", "zebra path:a/", "über", "x.y",
];

fn vault_at(p: &Path) -> Vault {
    Vault::open(Arc::new(StdFs::new(p, TrashMode::Vault).unwrap())).unwrap()
}

fn gen_content(r: &mut Rng) -> String {
    let mut s = String::new();
    if r.pct(25) {
        let tags = ["plants", "home", "plants/herbs", "Work"];
        s.push_str(&format!("---\ntitle: T{}\ntags: [{}]\n---\n", r.below(100), r.pick(&tags)));
    }
    let lines = 1 + r.below(6);
    for _ in 0..lines {
        match r.below(8) {
            0 => s.push_str(&format!("# {} {}\n", r.pick(NAMES), r.pick(WORDS))),
            1 => s.push_str(&format!("See [[{}]] and [[{}/{}|alias]]\n", r.pick(NAMES), r.pick(FOLDERS), r.pick(NAMES))),
            2 => s.push_str(&format!("[md]({}.md) ![img](pic.png) #{}\n", r.pick(NAMES).replace(' ', "%20"), ["plants", "home", "plants/herbs"][r.below(3)])),
            3 => s.push_str(&format!("`[[{}]]` code and ![[{}#H]]\n", r.pick(NAMES), r.pick(NAMES))),
            _ => {
                let n = 1 + r.below(8);
                let words: Vec<&str> = (0..n).map(|_| *r.pick(WORDS)).collect();
                s.push_str(&words.join(" "));
                s.push('\n');
            }
        }
    }
    s
}

fn note_paths(v: &Vault) -> Vec<String> {
    let mut p: Vec<String> = v
        .entries()
        .into_iter()
        .filter(|e| e.kind == EntryKind::File && cairn_core::path::is_markdown(&e.path))
        .map(|e| e.path)
        .collect();
    p.sort();
    p
}

fn dir_paths(v: &Vault) -> Vec<String> {
    let mut p: Vec<String> = v.entries().into_iter().filter(|e| e.kind == EntryKind::Dir).map(|e| e.path).collect();
    p.sort();
    p
}

/// Everything observable about the index, as comparable strings.
fn snapshot(v: &Vault) -> BTreeMap<String, String> {
    snapshot_opts(v, true)
}

fn snapshot_opts(v: &Vault, with_search: bool) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let mut entries: Vec<String> = v
        .entries()
        .into_iter()
        .map(|e| match e.kind {
            EntryKind::Dir => format!("{} dir", e.path),
            EntryKind::File => format!("{} file {} {}", e.path, e.size, e.mtime),
        })
        .collect();
    entries.sort();
    m.insert("entries".into(), entries.join("\n"));
    let notes = note_paths(v);
    for p in &notes {
        m.insert(format!("outgoing {p}"), format!("{:?}", v.outgoing(p)));
        m.insert(format!("backlinks {p}"), format!("{:?}", v.backlinks(p)));
        m.insert(format!("info {p}"), format!("{:?}", v.note_info(p)));
        for t in ["Note", "a/Note", "Café", "Leaf", "x.y", "pic.png"] {
            m.insert(format!("resolve {t} from {p}"), format!("{:?}", v.resolve(t, p)));
        }
    }
    m.insert("tags".into(), format!("{:?}", v.tags()));
    m.insert("graph".into(), format!("{:?}", v.graph(true)));
    m.insert("graph-resolved".into(), format!("{:?}", v.graph(false)));
    for q in QUERIES.iter().filter(|_| with_search) {
        let hits: Vec<String> = v
            .search(q, 1000)
            .into_iter()
            .map(|h| format!("{} {:.4} {:?}", h.path, h.score, h.snippets))
            .collect();
        m.insert(format!("search {q}"), hits.join("\n"));
    }
    m
}

fn diff(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Option<String> {
    for (k, va) in a {
        match b.get(k) {
            None => return Some(format!("only in incremental: {k}")),
            Some(vb) if vb != va => return Some(format!("{k}\n  incremental: {va}\n  fresh:       {vb}")),
            _ => {}
        }
    }
    for k in b.keys() {
        if !a.contains_key(k) {
            return Some(format!("only in fresh: {k}"));
        }
    }
    None
}

struct Run {
    dir: tempfile::TempDir,
    v: Vault,
    r: Rng,
    clock: u64,
    log: Vec<String>,
}

impl Run {
    fn abs(&self, p: &str) -> std::path::PathBuf {
        self.dir.path().join(p)
    }

    /// Give externally written files a distinct mtime so a change is never
    /// hidden by an identical (size, mtime) pair.
    fn touch(&mut self, p: &str) {
        self.clock += 7;
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000 + self.clock);
        if let Ok(f) = fs::File::options().write(true).open(self.abs(p)) {
            let _ = f.set_modified(t);
        }
    }

    fn rescan_after(&mut self, hints: &[String]) {
        if self.r.pct(50) {
            self.log.push("  -> rescan()".into());
            self.v.rescan().unwrap();
        } else {
            self.log.push(format!("  -> rescan_paths({hints:?})"));
            self.v.rescan_paths(hints).unwrap();
        }
    }

    /// True if new path `p`, or a folder on its way, differs only in case
    /// from an entry of the vault, on a file system that takes it for that
    /// entry. Ops that would make such a path are skipped there.
    fn clashes(&self, p: &str) -> bool {
        if CASE_SENSITIVE {
            return false;
        }
        let have: Vec<String> = self.v.entries().into_iter().map(|e| e.path).collect();
        let parts: Vec<&str> = p.split('/').collect();
        (1..=parts.len()).any(|n| {
            let q = parts[..n].join("/");
            have.iter().any(|h| *h != q && h.to_lowercase() == q.to_lowercase())
        })
    }

    fn rand_new_note_path(&mut self) -> String {
        let folder = *self.r.pick(FOLDERS);
        let name = format!("{}{}.md", self.r.pick(NAMES), if self.r.pct(30) { format!(" {}", self.r.below(5)) } else { String::new() });
        cairn_core::path::join(folder, &name)
    }

    fn step(&mut self) {
        let notes = note_paths(&self.v);
        let dirs = dir_paths(&self.v);
        let op = self.r.below(19);
        match op {
            0 | 1 => {
                let p = self.rand_new_note_path();
                if self.clashes(&p) {
                    return;
                }
                let c = gen_content(&mut self.r);
                self.log.push(format!("create_note {p:?}"));
                let _ = self.v.create_note(&p, &c);
            }
            2 | 3 if !notes.is_empty() => {
                let p = self.r.pick(&notes).clone();
                let c = gen_content(&mut self.r);
                self.log.push(format!("write_note {p:?}"));
                self.v.write_note(&p, &c, None).unwrap();
            }
            4 if !notes.is_empty() => {
                let from = self.r.pick(&notes).clone();
                let to = self.rand_new_note_path();
                if self.clashes(&to) {
                    return;
                }
                let parent = cairn_core::path::parent(&to).to_string();
                let _ = self.v.ensure_folder(&parent);
                self.log.push(format!("rename {from:?} -> {to:?}"));
                let _ = self.v.rename(&from, &to);
            }
            5 if !dirs.is_empty() => {
                let from = self.r.pick(&dirs).clone();
                let to = cairn_core::path::join(cairn_core::path::parent(&from), &format!("{} {}", self.r.pick(NAMES), self.r.below(9)));
                if self.clashes(&to) {
                    return;
                }
                self.log.push(format!("rename folder {from:?} -> {to:?}"));
                let _ = self.v.rename(&from, &to);
            }
            6 if !notes.is_empty() => {
                let p = self.r.pick(&notes).clone();
                self.log.push(format!("delete {p:?}"));
                self.v.delete(&p).unwrap();
            }
            7 if !dirs.is_empty() && self.r.pct(30) => {
                let p = self.r.pick(&dirs).clone();
                self.log.push(format!("delete folder {p:?}"));
                self.v.delete(&p).unwrap();
            }
            8 => {
                let folder = *self.r.pick(FOLDERS);
                let p = cairn_core::path::join(folder, if self.r.pct(50) { "pic.png" } else { "doc.pdf" });
                if self.clashes(&p) {
                    return;
                }
                self.log.push(format!("create_file {p:?}"));
                let _ = self.v.create_file(&p, &[1, 2, 3, self.r.below(200) as u8]);
            }
            9 | 10 => {
                // external create or edit
                let p = if !notes.is_empty() && self.r.pct(60) { self.r.pick(&notes).clone() } else { self.rand_new_note_path() };
                if self.clashes(&p) {
                    return;
                }
                let c = gen_content(&mut self.r);
                self.log.push(format!("external write {p:?}"));
                fs::create_dir_all(self.abs(cairn_core::path::parent(&p))).unwrap();
                fs::write(self.abs(&p), c).unwrap();
                self.touch(&p);
                self.rescan_after(&[p]);
            }
            11 if !notes.is_empty() => {
                // external rename (sometimes with an edit)
                let from = self.r.pick(&notes).clone();
                let to = self.rand_new_note_path();
                if self.abs(&to).exists() || self.clashes(&to) {
                    return;
                }
                fs::create_dir_all(self.abs(cairn_core::path::parent(&to))).unwrap();
                let edit = self.r.pct(30);
                self.log.push(format!("external rename {from:?} -> {to:?} edit={edit}"));
                fs::rename(self.abs(&from), self.abs(&to)).unwrap();
                if edit {
                    let c = gen_content(&mut self.r);
                    fs::write(self.abs(&to), c).unwrap();
                    self.touch(&to);
                }
                self.rescan_after(&[from, to]);
            }
            12 if !dirs.is_empty() => {
                // external folder rename
                let from = self.r.pick(&dirs).clone();
                let to = cairn_core::path::join(cairn_core::path::parent(&from), &format!("{} x{}", self.r.pick(NAMES), self.r.below(9)));
                if self.abs(&to).exists() || self.clashes(&to) {
                    return;
                }
                self.log.push(format!("external folder rename {from:?} -> {to:?}"));
                fs::rename(self.abs(&from), self.abs(&to)).unwrap();
                self.rescan_after(&[from, to]);
            }
            13 if !notes.is_empty() => {
                let p = self.r.pick(&notes).clone();
                self.log.push(format!("external delete {p:?}"));
                fs::remove_file(self.abs(&p)).unwrap();
                self.rescan_after(&[p]);
            }
            14 if !notes.is_empty() => {
                // in-app case-only rename (only when no case variant exists)
                let from = self.r.pick(&notes).clone();
                let name = cairn_core::path::file_name(&from);
                let flipped: String = name.chars().map(|c| if c.is_uppercase() { c.to_lowercase().next().unwrap() } else { c.to_uppercase().next().unwrap() }).collect();
                let to = cairn_core::path::join(cairn_core::path::parent(&from), &flipped.replace(".MD", ".md"));
                if to == from || notes.iter().any(|n| n.to_lowercase() == to.to_lowercase() && *n != from) {
                    return;
                }
                self.log.push(format!("case rename {from:?} -> {to:?}"));
                let _ = self.v.rename(&from, &to);
            }
            15 => {
                // external nested tree created at once, hinted by its top folder only
                let top = format!("{} t{}", self.r.pick(NAMES), self.r.below(9));
                if self.abs(&top).exists() {
                    return;
                }
                let sub = format!("{top}/{}", self.r.pick(NAMES));
                if self.clashes(&top) {
                    return;
                }
                fs::create_dir_all(self.abs(&sub)).unwrap();
                for (i, dir) in [top.clone(), sub.clone()].iter().enumerate() {
                    let p = format!("{dir}/{}{i}.md", self.r.pick(NAMES));
                    let c = gen_content(&mut self.r);
                    fs::write(self.abs(&p), c).unwrap();
                    self.touch(&p);
                }
                fs::write(self.abs(&format!("{sub}/pic.png")), [9u8, 9, 9]).unwrap();
                self.log.push(format!("external tree {top:?}"));
                self.rescan_after(&[top]);
            }
            16 if !notes.is_empty() => {
                // external rename seen as two separate watcher batches
                let from = self.r.pick(&notes).clone();
                let to = self.rand_new_note_path();
                if self.abs(&to).exists() || self.clashes(&to) {
                    return;
                }
                fs::create_dir_all(self.abs(cairn_core::path::parent(&to))).unwrap();
                fs::rename(self.abs(&from), self.abs(&to)).unwrap();
                self.log.push(format!("external rename in two batches {from:?} -> {to:?}"));
                self.v.rescan_paths(&[from]).unwrap();
                self.v.rescan_paths(&[to]).unwrap();
            }
            17 if !notes.is_empty() => {
                // external edit keeping the size, new mtime
                let p = self.r.pick(&notes).clone();
                let old = fs::read(self.abs(&p)).unwrap();
                if old.is_empty() {
                    return;
                }
                let mut new = old.clone();
                let i = self.r.below(new.len());
                if !new[i].is_ascii_alphabetic() {
                    return;
                }
                new[i] = if new[i] == b'z' { b'y' } else { b'z' };
                self.log.push(format!("external same-size edit {p:?}"));
                fs::write(self.abs(&p), new).unwrap();
                self.touch(&p);
                self.rescan_after(&[p]);
            }
            18 if !notes.is_empty() => {
                let p = self.r.pick(&notes).clone();
                let c = gen_content(&mut self.r);
                self.log.push(format!("write_file (md) {p:?}"));
                self.v.write_file(&p, c.as_bytes(), None).unwrap();
            }
            _ => {}
        }
    }
}

fn run_seed(seed: u64, steps: usize) -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    let v = vault_at(dir.path());
    let mut run = Run { dir, v, r: Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1), clock: 0, log: Vec::new() };
    for i in 0..steps {
        run.step();
        let with_search = i % 4 == 3 || i + 1 == steps;
        let inc = snapshot_opts(&run.v, with_search);
        let fresh = snapshot_opts(&vault_at(run.dir.path()), with_search);
        if let Some(d) = diff(&inc, &fresh) {
            let tail: Vec<&String> = run.log.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
            return Err(format!(
                "seed {seed} step {i}: incremental index differs from a fresh open\n{d}\nlast ops:\n{}",
                tail.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
            ));
        }
    }
    Ok(())
}

fn env_usize(k: &str, d: usize) -> usize {
    std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d)
}

#[test]
fn incremental_matches_fresh_random_ops() {
    let seeds = env_usize("ADV_INDEX_SEEDS", 3);
    let steps = env_usize("ADV_INDEX_STEPS", 100);
    let mut failures = Vec::new();
    for s in 1..=seeds as u64 {
        if let Err(e) = run_seed(s, steps) {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "{} of {seeds} seeds diverged:\n\n{}", failures.len(), failures.join("\n\n"));
}

fn vault_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let abs = d.path().join(p);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, c).unwrap();
    }
    let v = vault_at(d.path());
    (d, v)
}

/// Renaming a text file to `.md` in the file tree (allowed: the UI only adds
/// `.md` when the old name was a note) or receiving that rename from sync goes
/// through Vault::rename. The new note must be parsed: searchable, with its
/// links, tags and headings, and in the graph, as after a fresh open. Rename
/// keeps size and mtime, so no later rescan would fix a note left unparsed.
#[test]
fn finding_rename_changing_extension_txt_to_md() {
    let (d, v) = vault_with(&[("todo.txt", "buy zucchini\nsee [[Target]] #errand\n"), ("Target.md", "")]);
    v.rename("todo.txt", "todo.md").unwrap();
    let inc = snapshot(&v);
    let fresh = snapshot(&vault_at(d.path()));
    assert_eq!(v.search("zucchini", 10).len(), 1, "renamed note is searchable");
    assert_eq!(v.backlinks("Target.md").len(), 1, "its link shows in backlinks");
    assert!(diff(&inc, &fresh).is_none(), "{}", diff(&inc, &fresh).unwrap());
}

#[test]
fn finding_rename_changing_extension_md_to_txt() {
    let (d, v) = vault_with(&[("Note.md", "secret zucchini [[Target]] #tag"), ("Target.md", "")]);
    v.rename("Note.md", "Note.txt").unwrap();
    assert!(v.search("zucchini", 10).is_empty(), "a .txt file is not a note: {:?}", v.search("zucchini", 10));
    assert!(v.backlinks("Target.md").is_empty(), "{:?}", v.backlinks("Target.md"));
    assert!(v.graph(false).nodes.iter().all(|n| n.id != "Note.txt"));
    let inc = snapshot(&v);
    let fresh = snapshot(&vault_at(d.path()));
    assert!(diff(&inc, &fresh).is_none(), "{}", diff(&inc, &fresh).unwrap());
}
