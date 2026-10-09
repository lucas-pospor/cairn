//! What the page tells the backend about edits that are not on disk yet, so
//! that the backend can save them, or name them, when Windows ends the
//! session and the page cannot answer (session_end.rs).
//!
//! The page sends an entry for every note tab with unsaved edits: the hash
//! of the file the edits are based on, the tab's edit number (a counter that
//! grows with every change to any tab's text, also across page loads), what
//! keeps the note from being saved (a conflict with the disk, a failed
//! save) and, for a note without such a problem, its text as a save would
//! write it. It also says whether the last write of the settings failed.
//! Page saves (the write_note and recreate_note commands) report what they
//! wrote here too.
//!
//! [`write_pass`] writes the text held for each note with the same check a
//! save makes: only over the file the edits are based on, never over a
//! change made elsewhere, never a file that is gone. It never writes text
//! older than what Cairn already put on disk for that note. Cairn's own
//! writes (the page's and its own) are kept per note since the page's last
//! reset, so that an entry based on a file that Cairn itself replaced since
//! is written over the replacement ([`Held::translate`]): the page's save
//! landing first, or a second write by the backend after its first.
//! Whatever it cannot write is named for a refusal.
//!
//! This file uses no Tauri types: a Windows check can build it on its own.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use cairn_core::{CoreError, Vault};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// What keeps a note from being saved, as the page reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Problem {
    /// The file changed or went away on disk since the tab loaded it, and
    /// the user has not chosen what to keep.
    Conflict,
    /// The last save failed (read-only, locked, disk full).
    Failed,
}

/// A note tab with unsaved edits.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The hash of the file the edits are based on.
    pub base: String,
    /// The tab's edit number for `text`.
    pub edit: u64,
    pub problem: Option<Problem>,
    /// The note's text, or None when the page does not send it (a note too
    /// large, a text that is not well formed) or has a problem.
    pub text: Option<String>,
}

/// A write of a note by Cairn: by a page save, or by [`write_pass`].
#[derive(Clone, Debug, PartialEq)]
struct Write {
    /// The base hash it was written over (None: no check, "Keep mine").
    from: Option<String>,
    to: String,
    /// The edit number of the text written.
    edit: u64,
    by_backend: bool,
}

/// The writes kept per note: enough for a few writes in one session end.
const WRITES_KEPT: usize = 16;

/// Something the page cannot save, as the Unsaved changes dialog names it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Unsaved {
    /// The note's path; None for the settings.
    pub path: Option<String>,
    /// The note's title in quotes, or "the settings".
    pub name: String,
}

/// A note the backend wrote for the page: the file the page knew as `from`
/// is now `to`, with the text of edit number `edit`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Moved {
    pub path: String,
    pub from: String,
    pub to: String,
    pub edit: u64,
}

/// One request of the page (the session_hold command).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hold {
    /// Grows with every request; an older one that comes late is ignored.
    pub seq: u64,
    /// The open notebook's root as the page knows it (None: none open).
    pub root: Option<String>,
    /// Start over: the page loaded, or another notebook opened or closed.
    #[serde(default)]
    pub reset: bool,
    /// The answer to the backend's question `round`: every note with
    /// unsaved edits, which replaces what is held.
    #[serde(default)]
    pub round: Option<u64>,
    #[serde(default)]
    pub settings_failed: bool,
    /// Entries, each read on its own: one that cannot be read does not
    /// sink the others.
    #[serde(default)]
    pub notes: Vec<serde_json::Value>,
}

/// One entry of a [`Hold`]. A missing or null `text` means the text the
/// backend has for the same edit number, if any.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoteHold {
    path: String,
    #[serde(default)]
    release: bool,
    base: Option<String>,
    edit: Option<u64>,
    problem: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

/// What a request did.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldReply {
    /// Counters must start above this after a reset (see [`Held::hold`]).
    pub floor: u64,
    /// False when the request came too late or for another notebook.
    pub applied: bool,
}

/// The backend's copy of what the page has not saved, for one notebook.
#[derive(Default)]
pub struct Held {
    root: Option<String>,
    /// The notebook `root` names, as it was when the page reset: the
    /// backend writes into it only while it is still the open one.
    vault: Option<Arc<Vault>>,
    seq: u64,
    /// The highest edit number seen (and, after a reset, request number).
    max_edit: u64,
    /// The last answer taken, by round.
    answered: Option<u64>,
    settings_failed: bool,
    notes: BTreeMap<String, Entry>,
    writes: HashMap<String, Vec<Write>>,
    /// Page saves under way, by path.
    saving: HashMap<String, usize>,
    news: Vec<Moved>,
}

impl Held {
    /// Whether the page has said what it holds since it loaded.
    pub fn listening(&self) -> bool {
        self.seq > 0
    }

    /// Whether the page answered question `round`.
    pub fn answered(&self, round: u64) -> bool {
        self.answered == Some(round)
    }

    /// Takes a request of the page. `open` gives the open notebook's root,
    /// its vault and its root again (read in that order): a reset captures
    /// the vault only when both reads name the request's root, so that a
    /// notebook being switched is never taken for the one the page means.
    /// The reply's floor is above every number seen, for the page's
    /// counters after a reset (a clock set back must not make them go back).
    pub fn hold(&mut self, h: Hold, open: impl FnOnce() -> (Option<String>, Option<Arc<Vault>>, Option<String>)) -> HoldReply {
        let applied = if h.reset {
            let (r1, vault, r2) = open();
            let same = h.root.is_some() && r1 == h.root && r2 == h.root;
            let seen = self.seq.max(self.max_edit);
            *self = Held { root: h.root.clone(), vault: if same { vault } else { None }, max_edit: seen, ..Held::default() };
            true
        } else {
            h.seq > self.seq && h.root == self.root
        };
        if applied {
            self.seq = h.seq;
            self.settings_failed = h.settings_failed;
            if let Some(round) = h.round {
                self.notes.clear();
                self.answered = Some(round);
            }
            for v in h.notes {
                self.take(v);
            }
        }
        HoldReply { floor: self.seq.max(self.max_edit) + 1, applied }
    }

    /// One entry of a request.
    fn take(&mut self, v: serde_json::Value) {
        let n = match serde_json::from_value::<NoteHold>(v.clone()) {
            Ok(n) => n,
            Err(e) => {
                // Held without text, so that it still counts as unsaved.
                let Some(path) = v.get("path").and_then(|p| p.as_str()) else { return };
                log::warn!("session end: cannot take what the page holds for {path:?}: {e}");
                let edit = v.get("edit").and_then(|e| e.as_u64()).unwrap_or(0);
                self.notes.insert(path.to_string(), Entry { base: String::new(), edit, problem: None, text: None });
                return;
            }
        };
        if n.release {
            self.notes.remove(&n.path);
            return;
        }
        let (Some(base), Some(edit)) = (n.base, n.edit) else {
            self.notes.insert(n.path, Entry { base: String::new(), edit: 0, problem: None, text: None });
            return;
        };
        self.max_edit = self.max_edit.max(edit);
        let problem = match n.problem.as_deref() {
            None => None,
            Some("failed") => Some(Problem::Failed),
            Some(_) => Some(Problem::Conflict),
        };
        let text = match n.text {
            _ if problem.is_some() => None,
            Some(text) => Some(text),
            // The same text as before, if it is for the same edit: one edit
            // has one text. The page sends null for a note too large to send
            // while the user types, also just after its answer at the end of
            // the session gave the backend that very text.
            None => self.notes.get(&n.path).filter(|e| e.edit == edit).and_then(|e| e.text.clone()),
        };
        let entry = Entry { base, edit, problem, text };
        // The page saved this text or a newer one already.
        if problem.is_none() && self.superseded(&n.path, &entry) {
            self.notes.remove(&n.path);
            return;
        }
        self.notes.insert(n.path, entry);
    }

    /// Follows Cairn's writes of `path` from `base`, in the order they were
    /// made, through those `take` accepts. Returns the hash it ends at and
    /// the writes it went through. A write over no base, or a change made
    /// elsewhere, ends the chain.
    fn follow(&self, path: &str, base: &str, take: impl Fn(&Write) -> bool) -> (String, Vec<&Write>) {
        let mut at = base.to_string();
        let mut through = Vec::new();
        for w in self.writes.get(path).map(Vec::as_slice).unwrap_or_default() {
            if w.from.as_deref() == Some(at.as_str()) && take(w) {
                at = w.to.clone();
                through.push(w);
            }
        }
        (at, through)
    }

    /// The base to write `entry` over: the file its edits are based on, or
    /// what Cairn replaced that file with since, through writes of older
    /// text than the entry's.
    pub fn translate(&self, path: &str, entry: &Entry) -> String {
        self.follow(path, &entry.base, |w| w.edit < entry.edit).0
    }

    /// Whether Cairn has put the entry's text, or newer text based on the
    /// same file, on disk already.
    fn superseded(&self, path: &str, entry: &Entry) -> bool {
        self.follow(path, &entry.base, |_| true).1.iter().any(|w| w.edit >= entry.edit)
    }

    fn record(&mut self, path: &str, w: Write) {
        let list = self.writes.entry(path.to_string()).or_default();
        list.push(w);
        if list.len() > WRITES_KEPT {
            list.remove(0);
        }
    }

    /// A page save of `path` starts (see [`Held::saved`]).
    pub fn saving(&mut self, path: &str) {
        *self.saving.entry(path.to_string()).or_default() += 1;
    }

    /// A page save of `path` ended. When it wrote (`hash`) text of edit
    /// number `edit` into the held notebook over `base`, an entry for no
    /// newer text goes, and a newer one is now based on what it wrote.
    /// A write with no edit number (a plugin's) changes nothing here: the
    /// backend's own write then finds a change it must not overwrite.
    pub fn saved(&mut self, path: &str, vault: &Arc<Vault>, base: Option<&str>, edit: Option<u64>, hash: Option<&str>) {
        if let Some(n) = self.saving.get_mut(path) {
            *n -= 1;
            if *n == 0 {
                self.saving.remove(path);
            }
        }
        let (Some(edit), Some(hash)) = (edit, hash) else { return };
        if !self.vault.as_ref().is_some_and(|v| Arc::ptr_eq(v, vault)) {
            return;
        }
        self.max_edit = self.max_edit.max(edit);
        self.record(path, Write { from: base.map(String::from), to: hash.to_string(), edit, by_backend: false });
        match self.notes.get_mut(path) {
            Some(e) if e.edit <= edit => {
                self.notes.remove(path);
            }
            // Newer text stays based on the file the page knows; the write
            // just recorded leads from there to the file now on disk.
            Some(e) if e.problem == Some(Problem::Failed) => e.problem = None,
            _ => {}
        }
    }

    /// Whether a page save of `path` is under way.
    fn page_saving(&self, path: &str) -> bool {
        self.saving.contains_key(path)
    }

    /// settings.json was written.
    pub fn settings_written(&mut self) {
        self.settings_failed = false;
    }

    /// An entry renamed in Cairn: what is kept for it and for what is
    /// inside it follows.
    pub fn renamed(&mut self, from: &str, to: &str) {
        // Writes of a file that was at `to` before say nothing of what comes.
        self.deleted(to);
        let moved = |p: &str| -> Option<String> {
            if p == from {
                Some(to.to_string())
            } else {
                p.strip_prefix(from).filter(|rest| rest.starts_with('/')).map(|rest| format!("{to}{rest}"))
            }
        };
        let notes: Vec<String> = self.notes.keys().filter(|p| moved(p).is_some()).cloned().collect();
        for p in notes {
            if let (Some(e), Some(n)) = (self.notes.remove(&p), moved(&p)) {
                self.notes.insert(n, e);
            }
        }
        let writes: Vec<String> = self.writes.keys().filter(|p| moved(p).is_some()).cloned().collect();
        for p in writes {
            if let (Some(w), Some(n)) = (self.writes.remove(&p), moved(&p)) {
                self.writes.insert(n, w);
            }
        }
    }

    /// An entry deleted in Cairn: its writes no longer say anything.
    pub fn deleted(&mut self, path: &str) {
        self.writes.retain(|p, _| p != path && !p.strip_prefix(path).is_some_and(|rest| rest.starts_with('/')));
    }

    /// What the page reported it cannot save: notes with a problem and the
    /// settings. Notes waiting for autosave do not count.
    pub fn unsaved_by_the_page(&self) -> Vec<Unsaved> {
        let mut out: Vec<Unsaved> =
            self.notes.iter().filter(|(_, e)| e.problem.is_some()).map(|(p, _)| unsaved_note(p)).collect();
        if self.settings_failed {
            out.push(unsaved_settings());
        }
        out
    }

    /// Whether the page holds nothing unsaved at all.
    pub fn nothing_held(&self) -> bool {
        self.notes.is_empty() && !self.settings_failed
    }

    /// The file Cairn put in place of `base` of `path`, when the backend's
    /// own writes are among those that did (for a page save that found the
    /// file changed). None when that leads back to `base`.
    pub fn moved(&self, path: &str, base: &str) -> Option<String> {
        let (at, through) = self.follow(path, base, |_| true);
        (through.iter().any(|w| w.by_backend) && at != base).then_some(at)
    }

    /// What the backend wrote since the last call.
    pub fn take_news(&mut self) -> Vec<Moved> {
        std::mem::take(&mut self.news)
    }
}

/// A note named as the Unsaved changes dialog names it: its title in quotes.
fn unsaved_note(path: &str) -> Unsaved {
    let title = if cairn_core::path::is_markdown(path) {
        cairn_core::path::stem(path)
    } else {
        cairn_core::path::file_name(path)
    };
    Unsaved { path: Some(path.to_string()), name: format!("\"{title}\"") }
}

fn unsaved_settings() -> Unsaved {
    Unsaved { path: None, name: "the settings".into() }
}

/// How long to wait for a page save of the same note to end before a write
/// that found the file changed looks again.
const SAVE_POLL: Duration = Duration::from_millis(5);

/// Writes what `held` holds into `open`, the open notebook, and returns what
/// is left unsaved: notes with a problem, notes with no text, notes whose
/// write failed or found a change made elsewhere, notes of another notebook,
/// and the settings when their last write failed. Notes already on disk
/// count as saved. Runs on the calling thread and waits for the vault's
/// locks as long as they are held; `held` is never locked during a write.
pub fn write_pass(held: &Mutex<Held>, open: Option<Arc<Vault>>) -> Vec<Unsaved> {
    let (mut unsaved, todo, vault) = {
        let h = held.lock();
        let vault = h.vault.clone().filter(|v| open.as_ref().is_some_and(|o| Arc::ptr_eq(v, o)));
        let mut unsaved = Vec::new();
        let mut todo = Vec::new();
        for (path, e) in &h.notes {
            match (&e.problem, &e.text) {
                (Some(_), _) | (None, None) => unsaved.push(unsaved_note(path)),
                (None, Some(_)) if vault.is_none() => unsaved.push(unsaved_note(path)),
                (None, Some(_)) => {
                    if !h.superseded(path, e) {
                        todo.push(path.clone());
                    }
                }
            }
        }
        if h.settings_failed {
            unsaved.push(unsaved_settings());
        }
        (unsaved, todo, vault)
    };
    let Some(vault) = vault else { return unsaved };
    for path in todo {
        if !write_one(held, &vault, &path) {
            unsaved.push(unsaved_note(&path));
        }
    }
    unsaved
}

/// Writes the text held for `path`; true when it is on disk. It tries the
/// file Cairn put in place of the one the edits are based on, then that one
/// itself: either is written over only if it is what the disk holds.
fn write_one(held: &Mutex<Held>, vault: &Vault, path: &str) -> bool {
    let mut tried: Vec<String> = Vec::new();
    let (text, edit) = loop {
        // A page save of this note under way ends first: it may make this
        // write unneeded, or move its base.
        while held.lock().page_saving(path) {
            std::thread::sleep(SAVE_POLL);
        }
        let (text, edit, base) = {
            let h = held.lock();
            // The page saved it meanwhile, or let it go.
            let Some(entry) = h.notes.get(path) else { return true };
            if h.superseded(path, entry) {
                return true;
            }
            let Some(text) = entry.text.clone() else { return false };
            let candidates = [h.translate(path, entry), entry.base.clone()];
            match candidates.into_iter().find(|b| !tried.contains(b)) {
                Some(base) => (text, entry.edit, base),
                None => break (text, entry.edit),
            }
        };
        match vault.write_note(path, &text, Some(&base)) {
            Ok(r) => {
                log::info!("session end: wrote {path:?} (edit {edit})");
                let mut h = held.lock();
                h.record(path, Write { from: Some(base.clone()), to: r.hash.clone(), edit, by_backend: true });
                h.news.push(Moved { path: path.to_string(), from: base, to: r.hash, edit });
                return true;
            }
            Err(CoreError::Conflict(_)) => tried.push(base),
            Err(e) => {
                log::warn!("session end: could not write {path:?}: {e}");
                return false;
            }
        }
    };
    // A change made elsewhere, or the file is gone: saved only if the disk
    // holds this very text.
    let hash = cairn_core::index::hash_hex(&cairn_core::index::hash_bytes(text.as_bytes()));
    let on_disk = vault.read_file(path).map(|b| cairn_core::index::hash_hex(&cairn_core::index::hash_bytes(&b)));
    if on_disk.as_deref().is_ok_and(|d| *d == hash) {
        let mut h = held.lock();
        let from = tried.into_iter().next();
        h.record(path, Write { from, to: hash, edit, by_backend: false });
        return true;
    }
    log::warn!("session end: {path:?} changed on disk or is gone, so its edits are not written");
    false
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cairn_core::{StdFs, TrashMode};
    use serde_json::json;

    use super::*;

    struct Notebook {
        dir: PathBuf,
        vault: Arc<Vault>,
    }

    impl Drop for Notebook {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Notebook {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("cairn-held-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("A.md"), "a\n").unwrap();
            let fs = Arc::new(StdFs::new(&dir, TrashMode::Permanent).unwrap());
            Notebook { vault: Arc::new(Vault::open(fs).unwrap()), dir }
        }
        fn read(&self, path: &str) -> String {
            std::fs::read_to_string(self.dir.join(path)).unwrap()
        }
        fn hash(&self, path: &str) -> String {
            self.vault.read_note(path).unwrap().hash
        }
        fn root(&self) -> String {
            self.dir.to_string_lossy().into_owned()
        }
    }

    /// Held state for `nb`, after the page's reset.
    fn held_for(nb: &Notebook) -> Mutex<Held> {
        let mut h = Held::default();
        let root = Some(nb.root());
        let r = h.hold(Hold { seq: 1, root: root.clone(), reset: true, ..Hold::default() }, || {
            (root.clone(), Some(nb.vault.clone()), root.clone())
        });
        assert!(r.applied);
        Mutex::new(h)
    }

    fn hold(held: &Mutex<Held>, seq: u64, root: &str, notes: serde_json::Value) -> HoldReply {
        let notes = notes.as_array().unwrap().clone();
        held.lock().hold(Hold { seq, root: Some(root.into()), notes, ..Hold::default() }, || unreachable!())
    }

    fn names(u: &[Unsaved]) -> Vec<&str> {
        u.iter().map(|u| u.name.as_str()).collect()
    }

    #[test]
    fn held_text_is_written_over_the_file_it_is_based_on() {
        let nb = Notebook::new("plain");
        let held = held_for(&nb);
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": nb.hash("A.md"), "edit": 5, "text": "a\nmine" }]));
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert_eq!(nb.read("A.md"), "a\nmine");
        let news = held.lock().take_news();
        assert_eq!(news.len(), 1);
        assert_eq!((news[0].path.as_str(), news[0].to.as_str(), news[0].edit), ("A.md", nb.hash("A.md").as_str(), 5));
        // Written once: a second pass leaves it.
        std::fs::write(nb.dir.join("A.md"), "a\nmine").unwrap();
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert!(held.lock().take_news().is_empty());
    }

    #[test]
    fn a_change_made_elsewhere_is_never_overwritten() {
        let nb = Notebook::new("elsewhere");
        let held = held_for(&nb);
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": nb.hash("A.md"), "edit": 5, "text": "a\nmine" }]));
        std::fs::write(nb.dir.join("A.md"), "a\ntheirs\n").unwrap();
        assert_eq!(names(&write_pass(&held, Some(nb.vault.clone()))), ["\"A\""]);
        assert_eq!(nb.read("A.md"), "a\ntheirs\n");
    }

    #[test]
    fn a_change_that_is_this_very_text_counts_as_saved() {
        let nb = Notebook::new("same");
        let held = held_for(&nb);
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": nb.hash("A.md"), "edit": 5, "text": "a\nmine" }]));
        std::fs::write(nb.dir.join("A.md"), "a\nmine").unwrap();
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
    }

    #[test]
    fn a_note_gone_from_disk_is_not_written_again() {
        let nb = Notebook::new("gone");
        let held = held_for(&nb);
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": nb.hash("A.md"), "edit": 5, "text": "a\nmine" }]));
        std::fs::remove_file(nb.dir.join("A.md")).unwrap();
        assert_eq!(names(&write_pass(&held, Some(nb.vault.clone()))), ["\"A\""]);
        assert!(!nb.dir.join("A.md").exists());
    }

    #[test]
    fn a_write_that_fails_names_the_note() {
        let nb = Notebook::new("readonly");
        let held = held_for(&nb);
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": nb.hash("A.md"), "edit": 5, "text": "a\nmine" }]));
        let file = nb.dir.join("A.md");
        let mut perms = std::fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&file, perms.clone()).unwrap();
        let unsaved = write_pass(&held, Some(nb.vault.clone()));
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&file, perms).unwrap();
        assert_eq!(names(&unsaved), ["\"A\""]);
        assert_eq!(nb.read("A.md"), "a\n");
    }

    #[test]
    fn notes_with_a_problem_or_no_text_and_the_settings_are_named_and_never_written() {
        let nb = Notebook::new("problems");
        std::fs::write(nb.dir.join("B.md"), "b\n").unwrap();
        std::fs::write(nb.dir.join("C.md"), "c\n").unwrap();
        let held = held_for(&nb);
        let (a, b, c) = (nb.hash("A.md"), nb.hash("B.md"), nb.hash("C.md"));
        let reply = held.lock().hold(
            Hold {
                seq: 2,
                root: Some(nb.root()),
                settings_failed: true,
                notes: vec![
                    json!({ "path": "A.md", "base": a, "edit": 5, "problem": "conflict", "text": "x" }),
                    json!({ "path": "B.md", "base": b, "edit": 6, "problem": "failed" }),
                    json!({ "path": "C.md", "base": c, "edit": 7, "text": null }),
                ],
                ..Hold::default()
            },
            || unreachable!(),
        );
        assert!(reply.applied);
        assert_eq!(names(&held.lock().unsaved_by_the_page()), ["\"A\"", "\"B\"", "the settings"]);
        assert_eq!(names(&write_pass(&held, Some(nb.vault.clone()))), ["\"A\"", "\"B\"", "\"C\"", "the settings"]);
        assert_eq!((nb.read("A.md"), nb.read("B.md"), nb.read("C.md")), ("a\n".into(), "b\n".into(), "c\n".into()));
        held.lock().settings_written();
        assert_eq!(names(&held.lock().unsaved_by_the_page()), ["\"A\"", "\"B\""]);
    }

    #[test]
    fn a_second_write_goes_over_the_first() {
        // The held text was written; then the page's answer brings newer
        // text, still based on the file it loaded.
        let nb = Notebook::new("second");
        let held = held_for(&nb);
        let base = nb.hash("A.md");
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 5, "text": "a\none" }]));
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        let r = held.lock().hold(
            Hold {
                seq: 3,
                root: Some(nb.root()),
                round: Some(1),
                notes: vec![json!({ "path": "A.md", "base": base, "edit": 6, "text": "a\none two" })],
                ..Hold::default()
            },
            || unreachable!(),
        );
        assert!(r.applied && held.lock().answered(1));
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert_eq!(nb.read("A.md"), "a\none two");
        // The page's next save, still on the file it loaded, learns where its base went.
        assert_eq!(held.lock().moved("A.md", &base), Some(nb.hash("A.md")));
    }

    #[test]
    fn the_answers_text_stays_when_the_next_request_has_none() {
        // A note too large to send while the user types: only the page's
        // answer brings its text, and the page's next request, sent before
        // the answer's reply came back, says null for the same edit.
        let nb = Notebook::new("answer-kept");
        let held = held_for(&nb);
        let base = nb.hash("A.md");
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 5, "text": null }]));
        assert_eq!(names(&write_pass(&held, Some(nb.vault.clone()))), ["\"A\""]);
        let r = held.lock().hold(
            Hold {
                seq: 3,
                root: Some(nb.root()),
                round: Some(1),
                notes: vec![json!({ "path": "A.md", "base": base, "edit": 6, "text": "a\nlate" })],
                ..Hold::default()
            },
            || unreachable!(),
        );
        assert!(r.applied);
        assert!(hold(&held, 4, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 6, "text": null }])).applied);
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert_eq!(nb.read("A.md"), "a\nlate");
    }

    #[test]
    fn a_page_save_that_lands_first_moves_the_base_of_newer_text() {
        let nb = Notebook::new("page-first");
        let held = held_for(&nb);
        let base = nb.hash("A.md");
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 6, "text": "a\none two" }]));
        // The page's autosave of edit 5 goes through.
        held.lock().saving("A.md");
        let r = nb.vault.write_note("A.md", "a\none", Some(&base)).unwrap();
        held.lock().saved("A.md", &nb.vault, Some(&base), Some(5), Some(&r.hash));
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert_eq!(nb.read("A.md"), "a\none two");
    }

    #[test]
    fn older_text_never_goes_over_what_the_page_saved() {
        let nb = Notebook::new("older");
        let held = held_for(&nb);
        let base = nb.hash("A.md");
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 5, "text": "a\none" }]));
        // The page saved edit 6 before the backend wrote edit 5.
        held.lock().saving("A.md");
        let r = nb.vault.write_note("A.md", "a\none two", Some(&base)).unwrap();
        held.lock().saved("A.md", &nb.vault, Some(&base), Some(6), Some(&r.hash));
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert_eq!(nb.read("A.md"), "a\none two");
        // An entry for edit 5 that comes late is dropped.
        hold(&held, 3, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 5, "text": "a\none" }]));
        assert!(held.lock().nothing_held());
    }

    #[test]
    fn a_plugin_write_is_not_overwritten() {
        let nb = Notebook::new("plugin");
        let held = held_for(&nb);
        let base = nb.hash("A.md");
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 5, "text": "a\nmine" }]));
        // A plugin writes the note over the same base, with no edit number.
        held.lock().saving("A.md");
        let r = nb.vault.write_note("A.md", "a\nplugin", Some(&base)).unwrap();
        held.lock().saved("A.md", &nb.vault, Some(&base), None, Some(&r.hash));
        assert_eq!(names(&write_pass(&held, Some(nb.vault.clone()))), ["\"A\""]);
        assert_eq!(nb.read("A.md"), "a\nplugin");
    }

    #[test]
    fn a_page_save_under_way_ends_before_the_write() {
        let nb = Notebook::new("under-way");
        let held = Arc::new(held_for(&nb));
        let base = nb.hash("A.md");
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 6, "text": "a\none two" }]));
        held.lock().saving("A.md");
        let (h, v, b) = (held.clone(), nb.vault.clone(), base.clone());
        let page = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            let r = v.write_note("A.md", "a\none", Some(&b)).unwrap();
            h.lock().saved("A.md", &v, Some(&b), Some(5), Some(&r.hash));
        });
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        page.join().unwrap();
        assert_eq!(nb.read("A.md"), "a\none two");
    }

    #[test]
    fn nothing_is_written_into_another_notebook() {
        let nb = Notebook::new("first");
        let other = Notebook::new("second-notebook");
        let held = held_for(&nb);
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": nb.hash("A.md"), "edit": 5, "text": "a\nmine" }]));
        assert_eq!(names(&write_pass(&held, Some(other.vault.clone()))), ["\"A\""]);
        assert_eq!(names(&write_pass(&held, None)), ["\"A\""]);
        assert_eq!((nb.read("A.md"), other.read("A.md")), ("a\n".into(), "a\n".into()));
        // A reset while the notebook is switched captures no vault.
        let mut h = Held::default();
        let root = Some(nb.root());
        h.hold(Hold { seq: 1, root: root.clone(), reset: true, ..Hold::default() }, || {
            (root.clone(), Some(other.vault.clone()), Some(other.root()))
        });
        let held = Mutex::new(h);
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": nb.hash("A.md"), "edit": 5, "text": "a\nmine" }]));
        assert_eq!(names(&write_pass(&held, Some(nb.vault.clone()))), ["\"A\""]);
    }

    #[test]
    fn requests_that_come_late_or_for_another_notebook_are_ignored() {
        let nb = Notebook::new("late");
        let held = held_for(&nb);
        let base = nb.hash("A.md");
        assert!(hold(&held, 3, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 6, "text": "new" }])).applied);
        assert!(!hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 5, "text": "old" }])).applied);
        assert!(!hold(&held, 4, "elsewhere", json!([{ "path": "A.md", "release": true }])).applied);
        assert_eq!(held.lock().notes["A.md"].text.as_deref(), Some("new"));
        // A missing or null text keeps the text of the same edit, and a newer
        // edit without text has none.
        hold(&held, 5, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 6 }]));
        assert_eq!(held.lock().notes["A.md"].text.as_deref(), Some("new"));
        hold(&held, 6, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 6, "text": null }]));
        assert_eq!(held.lock().notes["A.md"].text.as_deref(), Some("new"));
        hold(&held, 7, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 7, "text": null }]));
        assert_eq!(held.lock().notes["A.md"].text, None);
        hold(&held, 8, &nb.root(), json!([{ "path": "A.md", "base": base, "edit": 8 }]));
        assert_eq!(held.lock().notes["A.md"].text, None);
        hold(&held, 9, &nb.root(), json!([{ "path": "A.md", "release": true }]));
        assert!(held.lock().nothing_held());
    }

    #[test]
    fn an_entry_that_cannot_be_read_still_counts() {
        let nb = Notebook::new("bad-entry");
        let held = held_for(&nb);
        let r = hold(&held, 2, &nb.root(), json!([{ "path": "B.md", "edit": "seven" }, { "nopath": 1 }, { "path": "A.md", "base": nb.hash("A.md"), "edit": 5, "text": "a\nmine" }]));
        assert!(r.applied);
        assert_eq!(names(&write_pass(&held, Some(nb.vault.clone()))), ["\"B\""]);
        assert_eq!(nb.read("A.md"), "a\nmine");
    }

    #[test]
    fn a_reset_starts_over_and_keeps_counters_above_what_was_seen() {
        let nb = Notebook::new("reset");
        let held = held_for(&nb);
        hold(&held, 50, &nb.root(), json!([{ "path": "A.md", "base": nb.hash("A.md"), "edit": 900, "text": "x" }]));
        let root = Some(nb.root());
        let r = held.lock().hold(Hold { seq: 10, root: root.clone(), reset: true, ..Hold::default() }, || {
            (root.clone(), Some(nb.vault.clone()), root.clone())
        });
        assert_eq!(r, HoldReply { floor: 901, applied: true });
        assert!(held.lock().nothing_held());
        assert!(held.lock().listening());
        assert!(!Held::default().listening());
    }

    #[test]
    fn renames_in_cairn_carry_what_is_held() {
        let nb = Notebook::new("rename");
        let held = held_for(&nb);
        let base = nb.hash("A.md");
        hold(&held, 2, &nb.root(), json!([
            { "path": "Dir/B.md", "base": base, "edit": 5, "problem": "failed" },
            { "path": "Dir2/C.md", "base": base, "edit": 6, "problem": "failed" },
        ]));
        held.lock().renamed("Dir", "New");
        assert_eq!(names(&held.lock().unsaved_by_the_page()), ["\"C\"", "\"B\""]);
        let paths: Vec<Option<String>> = held.lock().unsaved_by_the_page().into_iter().map(|u| u.path).collect();
        assert_eq!(paths, [Some("Dir2/C.md".to_string()), Some("New/B.md".to_string())]);
    }

    #[test]
    fn a_delete_in_cairn_forgets_the_writes_of_what_it_deleted() {
        let nb = Notebook::new("delete");
        let held = held_for(&nb);
        let base = nb.hash("A.md");
        held.lock().saving("Dir/A.md");
        held.lock().saved("Dir/A.md", &nb.vault, Some(&base), Some(5), Some("h1"));
        held.lock().saving("Dirt.md");
        held.lock().saved("Dirt.md", &nb.vault, Some(&base), Some(6), Some("h2"));
        held.lock().deleted("Dir");
        assert!(!held.lock().writes.contains_key("Dir/A.md"));
        assert!(held.lock().writes.contains_key("Dirt.md"));
    }

    /// A page save of `text` (edit `edit`) over what the disk holds.
    fn page_save(nb: &Notebook, held: &Mutex<Held>, text: &str, edit: u64) -> String {
        let base = nb.hash("A.md");
        held.lock().saving("A.md");
        let r = nb.vault.write_note("A.md", text, Some(&base)).unwrap();
        held.lock().saved("A.md", &nb.vault, Some(&base), Some(edit), Some(&r.hash));
        r.hash
    }

    #[test]
    fn saves_that_come_back_to_the_same_text_lead_nowhere_else() {
        // a, ab, a again: the first and the last file have the same hash.
        let nb = Notebook::new("cycle");
        let held = held_for(&nb);
        let h0 = nb.hash("A.md");
        page_save(&nb, &held, "a\nb", 5);
        page_save(&nb, &held, "a\nbc", 6);
        assert_eq!(page_save(&nb, &held, "a\n", 7), h0);
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": h0, "edit": 8, "text": "a\nd" }]));
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert_eq!(nb.read("A.md"), "a\nd");
        // A save the backend's write moved goes on from there, and a chain
        // that comes back to where it started leads nowhere.
        let h4 = nb.hash("A.md");
        assert_eq!(held.lock().moved("A.md", &h0), Some(h4.clone()));
        assert_eq!(held.lock().moved("A.md", &h4), None);
    }

    #[test]
    fn a_file_put_back_elsewhere_is_written_over_as_the_page_knows_it() {
        // Cairn saved b over a; another program put a back; the page took
        // that file and typed on.
        let nb = Notebook::new("put-back");
        let held = held_for(&nb);
        let h0 = nb.hash("A.md");
        page_save(&nb, &held, "a\nb", 5);
        std::fs::write(nb.dir.join("A.md"), "a\n").unwrap();
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": h0, "edit": 9, "text": "a\nc" }]));
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert_eq!(nb.read("A.md"), "a\nc");
    }

    #[test]
    fn a_save_of_another_file_at_the_same_path_says_nothing_of_this_one() {
        // A.md was saved at edit 15, then replaced outside Cairn by a note
        // whose tab holds older unsaved edits.
        let nb = Notebook::new("other-file");
        let held = held_for(&nb);
        page_save(&nb, &held, "a\nsaved", 15);
        std::fs::write(nb.dir.join("A.md"), "other\n").unwrap();
        let other = nb.hash("A.md");
        hold(&held, 2, &nb.root(), json!([{ "path": "A.md", "base": other, "edit": 12, "text": "other\nedit" }]));
        assert!(!held.lock().nothing_held());
        assert!(write_pass(&held, Some(nb.vault.clone())).is_empty());
        assert_eq!(nb.read("A.md"), "other\nedit");
    }

    #[test]
    fn names_are_titles_in_quotes() {
        assert_eq!(unsaved_note("Dir/My note.md").name, "\"My note\"");
        assert_eq!(unsaved_note("x.markdown").name, "\"x\"");
        assert_eq!(unsaved_note("data.csv").name, "\"data.csv\"");
    }
}
