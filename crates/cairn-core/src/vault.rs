//! A vault: the folder plus its index, and every operation that changes it.

use std::borrow::Borrow;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hash as StdHash;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::fs::{EntryKind, FileStat, VaultFs};
use crate::index::{self, Backlinks, Hash, Index, OutgoingLink, PreparedNote, SearchHit};
use crate::parse::LinkKind;
use rayon::prelude::*;
use crate::path as vpath;

/// A change to the vault, reported to the UI and (later) to sync.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Change {
    Created { entry: FileStat },
    Modified { entry: FileStat },
    /// For a folder, only the folder itself is reported; children went with it.
    Deleted { path: String, kind: EntryKind },
    /// For a folder, only the folder itself is reported; children moved with it.
    Renamed { from: String, entry: FileStat },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteContent {
    pub content: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResult {
    pub entry: FileStat,
    pub hash: String,
    pub changes: Vec<Change>,
}

/// Per-vault settings folder (hidden, so never indexed or shown).
pub const CONFIG_DIR: &str = ".cairn";

/// Why a name in [`Vault::skipped_backslash_names`] is not synced, as the
/// list of files not synced shows it.
pub const BACKSLASH_REASON: &str = "The name contains a backslash. Rename it to sync it.";

pub struct Vault {
    fs: Arc<dyn VaultFs>,
    index: RwLock<Index>,
    /// Serializes mutations and rescans so a scan never sees half an operation.
    op: Mutex<()>,
    /// Folders (and entries) the last scan could not read.
    unreadable: Mutex<Vec<String>>,
}

/// True if what the index has at `p` is unknown after a listing that
/// could not read `unreadable`: `p` is inside one of those folders, or is
/// one of those paths and was not `listed` (an entry that could not be
/// looked up). The index keeps such entries as they are.
fn unknown(p: &str, unreadable: &[String], listed: impl Fn(&str) -> bool) -> bool {
    unreadable.iter().any(|d| vpath::is_inside(p, d) || (p == d && !listed(p)))
}

/// True if `p` or a folder above it is in `dirs`.
fn same_or_inside_any<S: Borrow<str> + Eq + StdHash>(p: &str, dirs: &HashSet<S>) -> bool {
    let mut p = p;
    while !p.is_empty() {
        if dirs.contains(p) {
            return true;
        }
        p = vpath::parent(p);
    }
    false
}

/// Pair deleted and created files with the same content `key` as renames;
/// returns (deleted, created) index pairs in deleted order.
///
/// A content that one deleted and one created file share is a rename. When
/// more files share it (notes made from one template), a pair needs the
/// same file name, or else the same folder, found once on each side; the
/// files left over are deleted and created, not guessed.
pub fn pair_renames<K: Eq + StdHash>(deleted: &[(&str, K)], created: &[(&str, K)]) -> Vec<(usize, usize)> {
    fn by_part<'a, K>(part: fn(&str) -> &str, files: &[(&'a str, K)], ix: &[usize]) -> HashMap<&'a str, Vec<usize>> {
        let mut m: HashMap<&'a str, Vec<usize>> = HashMap::new();
        for &i in ix {
            m.entry(part(files[i].0)).or_default().push(i);
        }
        m
    }
    let mut groups: HashMap<&K, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for (i, (_, k)) in deleted.iter().enumerate() {
        groups.entry(k).or_default().0.push(i);
    }
    for (i, (_, k)) in created.iter().enumerate() {
        if let Some(g) = groups.get_mut(k) {
            g.1.push(i);
        }
    }
    let mut out = Vec::new();
    for (mut ds, mut cs) in groups.into_values() {
        for part in [vpath::file_name as fn(&str) -> &str, vpath::parent] {
            if ds.is_empty() || cs.is_empty() || ds.len() == 1 && cs.len() == 1 {
                break;
            }
            let by_c = by_part(part, created, &cs);
            let mut pairs = Vec::new();
            for (p, dv) in by_part(part, deleted, &ds) {
                if let ([d], Some([c])) = (dv.as_slice(), by_c.get(p).map(Vec::as_slice)) {
                    pairs.push((*d, *c));
                }
            }
            let (pd, pc): (HashSet<usize>, HashSet<usize>) = pairs.iter().copied().unzip();
            ds.retain(|d| !pd.contains(d));
            cs.retain(|c| !pc.contains(c));
            out.extend(pairs);
        }
        if let ([d], [c]) = (ds.as_slice(), cs.as_slice()) {
            out.push((*d, *c));
        }
    }
    out.sort_unstable();
    out
}

pub fn parse_hash(hex: &str) -> Option<Hash> {
    // ASCII first: slicing by bytes would panic inside a multi-byte character.
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn decode_utf8(path: &str, bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|_| CoreError::Io(format!("{path}: not valid UTF-8 text")))
}

impl Vault {
    /// Open a vault and build the index from the files.
    pub fn open(fs: Arc<dyn VaultFs>) -> Result<Vault> {
        let v = Vault { fs, index: RwLock::new(Index::default()), op: Mutex::new(()), unreadable: Mutex::new(Vec::new()) };
        {
            let _g = v.op.lock();
            let (listing, unreadable) = v.fs.list_partial("")?;
            *v.unreadable.lock() = unreadable;
            // Read, hash, parse and tokenize in parallel; insert sequentially.
            let prepared: Vec<(FileStat, Option<(PreparedNote, Hash)>)> = listing
                .into_par_iter()
                .map(|st| {
                    if st.kind == EntryKind::File && vpath::is_markdown(&st.path) {
                        match v.fs.read(&st.path) {
                            Ok(bytes) => {
                                let h = index::hash_bytes(&bytes);
                                let text = String::from_utf8_lossy(&bytes).into_owned();
                                let p = PreparedNote::new(&st.path, text);
                                (st, Some((p, h)))
                            }
                            Err(e) => {
                                log::warn!("cannot read {}: {e}", st.path);
                                (st, None)
                            }
                        }
                    } else {
                        (st, None)
                    }
                })
                .collect();
            let mut idx = v.index.write();
            for (st, note) in prepared {
                match note {
                    Some((p, h)) => idx.put_prepared(st, p, h),
                    None => idx.put_entry(st, None),
                }
            }
        }
        Ok(v)
    }

    pub fn fs(&self) -> &Arc<dyn VaultFs> {
        &self.fs
    }

    /// Read access to the index (keep the guard short-lived).
    pub fn index(&self) -> RwLockReadGuard<'_, Index> {
        self.index.read()
    }

    /// Read a file and put it in the index. Unreadable notes are still listed.
    fn load_into(&self, idx: &mut Index, st: FileStat) {
        if st.kind == EntryKind::File && vpath::is_markdown(&st.path) {
            match self.fs.read(&st.path) {
                Ok(bytes) => {
                    let h = index::hash_bytes(&bytes);
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    idx.put_note(st, text, h);
                }
                Err(e) => {
                    log::warn!("cannot read {}: {e}", st.path);
                    idx.put_entry(st, None);
                }
            }
        } else {
            idx.put_entry(st, None);
        }
    }

    pub fn entries(&self) -> Vec<FileStat> {
        self.index.read().entries().cloned().collect()
    }

    /// Folders the last scan could not read (no permission), and entries it
    /// could not look up. What is there is unknown, not deleted: a rescan
    /// keeps what the index had there.
    pub fn unreadable_folders(&self) -> Vec<String> {
        self.unreadable.lock().clone()
    }

    /// Files and folders the scans leave out because their name has a
    /// backslash (see [`VaultFs::skipped_backslash_names`]): their paths on
    /// disk relative to the vault folder, such as `notes/a\b.md`. They are
    /// not shown, indexed or synced; renaming them outside Cairn brings
    /// them in. Sync lists them as not synced, with [`BACKSLASH_REASON`].
    pub fn skipped_backslash_names(&self) -> Vec<String> {
        self.fs.skipped_backslash_names()
    }

    /// Read a note. Hidden paths (`.cairn/`, `.git/`, `.trash/`, ...) are
    /// refused like in every other entry point: they are not part of the vault
    /// the UI and plugins see. Config goes through `read_config`.
    pub fn read_note(&self, path: &str) -> Result<NoteContent> {
        let path = Self::visible_path(path)?;
        let bytes = self.fs.read(&path)?;
        let hash = index::hash_hex(&index::hash_bytes(&bytes));
        Ok(NoteContent { content: decode_utf8(&path, bytes)?, hash })
    }

    /// Read any file that is not hidden (see `read_note`).
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        self.fs.read(&Self::visible_path(path)?)
    }

    /// `path` normalized, or `InvalidPath` if it is inside a dot-folder or a
    /// dotfile.
    fn visible_path(path: &str) -> Result<String> {
        let path = vpath::normalize(path)?;
        if vpath::is_hidden(&path) {
            return Err(CoreError::InvalidPath(path));
        }
        Ok(path)
    }

    /// Refuse `path` if a symlink leads it outside the vault's notes on disk
    /// (out of the vault, into a hidden folder, or from a note to a file
    /// that is not one), for callers that must stay inside (plugins). The
    /// app itself follows the links the user made.
    pub fn check_in_vault(&self, path: &str) -> Result<()> {
        let path = vpath::normalize(path)?;
        if self.fs.leads_outside(&path) {
            return Err(CoreError::InvalidPath(format!("{path} leads outside the notebook's notes")));
        }
        Ok(())
    }

    fn check_new_path(path: &str) -> Result<()> {
        if path.is_empty() || vpath::is_hidden(path) {
            return Err(CoreError::InvalidPath(path.to_string()));
        }
        vpath::validate_name(vpath::file_name(path))
    }

    /// `check_new_path` for the new path of a rename, where a device name
    /// is taken: sync applies renames from other devices with `rename`.
    fn check_moved_path(path: &str) -> Result<()> {
        if path.is_empty() || vpath::is_hidden(path) {
            return Err(CoreError::InvalidPath(path.to_string()));
        }
        vpath::validate_any_name(vpath::file_name(path))
    }

    /// Refuse a new `path` when a folder that creating it would add on the
    /// way (`bad:dir` in `bad:dir/x.md`) has a name `validate_name` rejects.
    /// Folders that exist keep whatever name they have.
    fn check_new_folders(&self, idx: &Index, path: &str) -> Result<()> {
        let mut p = vpath::parent(path);
        while !p.is_empty() && idx.entry(p).is_none() && self.fs.stat(p)?.is_none() {
            vpath::validate_name(vpath::file_name(p))?;
            p = vpath::parent(p);
        }
        Ok(())
    }

    /// Refuse a new `path` when it, or a missing folder on the way to it,
    /// differs only in case from an existing sibling (`Note.md` next to
    /// `note.md`): those collide on case-insensitive file systems. `own`,
    /// the entry being renamed to `path`, does not count. The error names
    /// the existing entry.
    fn check_case_twins(idx: &Index, path: &str, own: Option<&str>) -> Result<()> {
        for end in path.match_indices('/').map(|(i, _)| i).chain([path.len()]) {
            let p = &path[..end];
            if idx.entry(p).is_some() {
                continue;
            }
            // Nothing below a new folder exists yet, so one check is enough.
            return match idx.case_twin(p, own) {
                Some(twin) => Err(CoreError::AlreadyExists(twin.to_string())),
                None => Ok(()),
            };
        }
        Ok(())
    }

    /// Create missing parent folders of `path`, both on disk and in the index.
    fn ensure_parents(&self, idx: &mut Index, path: &str, changes: &mut Vec<Change>) -> Result<()> {
        let parent = vpath::parent(path);
        if parent.is_empty() || idx.entry(parent).is_some_and(|e| e.kind == EntryKind::Dir) {
            return Ok(());
        }
        self.fs.create_dir(parent)?;
        let mut missing = Vec::new();
        let mut p = parent;
        while !p.is_empty() && idx.entry(p).is_none() {
            missing.push(p.to_string());
            p = vpath::parent(p);
        }
        for d in missing.into_iter().rev() {
            if let Some(st) = self.fs.stat(&d)? {
                idx.put_entry(st.clone(), None);
                changes.push(Change::Created { entry: st });
            }
        }
        Ok(())
    }

    /// Create a new note. Missing parent folders are created.
    pub fn create_note(&self, path: &str, content: &str) -> Result<WriteResult> {
        let path = vpath::normalize(path)?;
        Self::check_new_path(&path)?;
        if !vpath::is_markdown(&path) {
            return Err(CoreError::NotANote(path));
        }
        let _g = self.op.lock();
        if self.fs.stat(&path)?.is_some() {
            return Err(CoreError::AlreadyExists(path));
        }
        let mut idx = self.index.write();
        Self::check_case_twins(&idx, &path, None)?;
        self.check_new_folders(&idx, &path)?;
        let mut changes = Vec::new();
        self.ensure_parents(&mut idx, &path, &mut changes)?;
        let st = self.fs.write(&path, content.as_bytes())?;
        let h = index::hash_bytes(content.as_bytes());
        idx.put_note(st.clone(), content.to_string(), h);
        changes.push(Change::Created { entry: st.clone() });
        Ok(WriteResult { entry: st, hash: index::hash_hex(&h), changes })
    }

    /// Create a binary file (attachment). Fails if it exists.
    pub fn create_file(&self, path: &str, data: &[u8]) -> Result<WriteResult> {
        let path = vpath::normalize(path)?;
        Self::check_new_path(&path)?;
        self.new_file(path, data, true)
    }

    /// Write a file only where there is none: `CoreError::AlreadyExists` if
    /// anything is at `path` on disk. Unlike `create_file`, any name is
    /// taken, as with `write_file`: sync uses it for files from other
    /// devices, at a path that was free when it last looked. Nor is a case
    /// twin refused (a case-sensitive device keeps `Note.md` next to
    /// `note.md` from another device); sync deals with those itself.
    pub fn write_new_file(&self, path: &str, data: &[u8]) -> Result<WriteResult> {
        let path = vpath::normalize(path)?;
        if path.is_empty() || vpath::is_hidden(&path) {
            return Err(CoreError::InvalidPath(path));
        }
        self.new_file(path, data, false)
    }

    /// `write_new_file`; for a file the user creates (`checked`), also
    /// refuse a case twin and new folders with names `validate_name`
    /// rejects.
    fn new_file(&self, path: String, data: &[u8], checked: bool) -> Result<WriteResult> {
        let _g = self.op.lock();
        if self.fs.stat(&path)?.is_some() {
            return Err(CoreError::AlreadyExists(path));
        }
        let mut idx = self.index.write();
        if checked {
            Self::check_case_twins(&idx, &path, None)?;
            self.check_new_folders(&idx, &path)?;
        }
        let mut changes = Vec::new();
        self.ensure_parents(&mut idx, &path, &mut changes)?;
        let st = self.fs.write(&path, data)?;
        let h = index::hash_bytes(data);
        if vpath::is_markdown(&path) {
            idx.put_note(st.clone(), String::from_utf8_lossy(data).into_owned(), h);
        } else {
            idx.put_entry(st.clone(), None);
        }
        changes.push(Change::Created { entry: st.clone() });
        Ok(WriteResult { entry: st, hash: index::hash_hex(&h), changes })
    }

    /// Write `data` to `path`. With `base_hash`, only if the file on disk
    /// still has that content hash and does not change while `data` is
    /// written (another program's save wins): `CoreError::Conflict`
    /// otherwise. Missing parent folders are created.
    fn put(&self, idx: &mut Index, path: &str, data: &[u8], base_hash: Option<&str>, changes: &mut Vec<Change>) -> Result<FileStat> {
        let Some(base) = base_hash else {
            self.ensure_parents(idx, path, changes)?;
            return self.fs.write(path, data);
        };
        let base = parse_hash(base).ok_or_else(|| CoreError::InvalidPath("bad hash".into()))?;
        // Always the bytes on disk, not the indexed hash: a same-size edit
        // can keep the indexed size and mtime (coarse mtimes, cp -p).
        let st = self.fs.write_if(path, data, &|cur| index::hash_bytes(cur) == base)?;
        // The file was there, so its folders are: this only indexes them.
        self.ensure_parents(idx, path, changes)?;
        Ok(st)
    }

    /// Save a note. If `base_hash` is given, the write only happens when the
    /// file on disk still has that hash; otherwise `CoreError::Conflict`.
    /// Without one, a note that is not on disk is created, and like
    /// `create_note` its name and those of the folders it adds must pass
    /// `validate_name`. Files already on disk keep whatever name they have
    /// (`recreate_note` brings back a deleted one under its old name).
    pub fn write_note(&self, path: &str, content: &str, base_hash: Option<&str>) -> Result<WriteResult> {
        let path = Self::visible_path(path)?;
        if !vpath::is_markdown(&path) {
            return Err(CoreError::NotANote(path));
        }
        let _g = self.op.lock();
        let mut idx = self.index.write();
        // With a base hash a missing file is a conflict, never created.
        if base_hash.is_none() && self.fs.stat(&path)?.is_none() {
            vpath::validate_name(vpath::file_name(&path))?;
            self.check_new_folders(&idx, &path)?;
        }
        self.store_note(&mut idx, path, content, base_hash)
    }

    /// Save a note whose file is gone (deleted or moved while it had unsaved
    /// edits). Fails with `CoreError::Conflict` if something is at `path`
    /// again, so a file that came back is never replaced blindly. Unlike
    /// `create_note`, the name is not checked: it was a note already.
    pub fn recreate_note(&self, path: &str, content: &str) -> Result<WriteResult> {
        let path = Self::visible_path(path)?;
        if !vpath::is_markdown(&path) {
            return Err(CoreError::NotANote(path));
        }
        let _g = self.op.lock();
        let mut idx = self.index.write();
        if self.fs.stat(&path)?.is_some() {
            return Err(CoreError::Conflict(path));
        }
        self.store_note(&mut idx, path, content, None)
    }

    /// Write a note (and missing parent folders) and index it. With a
    /// `base_hash` the write only happens when the file on disk still has it.
    fn store_note(&self, idx: &mut Index, path: String, content: &str, base_hash: Option<&str>) -> Result<WriteResult> {
        let mut changes = Vec::new();
        let existed = idx.entry(&path).is_some();
        let st = self.put(idx, &path, content.as_bytes(), base_hash, &mut changes)?;
        let h = index::hash_bytes(content.as_bytes());
        idx.put_note(st.clone(), content.to_string(), h);
        changes.push(if existed {
            Change::Modified { entry: st.clone() }
        } else {
            Change::Created { entry: st.clone() }
        });
        Ok(WriteResult { entry: st, hash: index::hash_hex(&h), changes })
    }

    /// Overwrite (or create) any file. With `base_hash`, only if the file on
    /// disk still has that content hash, like `write_note`.
    pub fn write_file(&self, path: &str, data: &[u8], base_hash: Option<&str>) -> Result<WriteResult> {
        let path = vpath::normalize(path)?;
        if path.is_empty() || vpath::is_hidden(&path) {
            return Err(CoreError::InvalidPath(path));
        }
        let _g = self.op.lock();
        let mut idx = self.index.write();
        let mut changes = Vec::new();
        let existed = idx.entry(&path).is_some();
        let st = self.put(&mut idx, &path, data, base_hash, &mut changes)?;
        let h = index::hash_bytes(data);
        if vpath::is_markdown(&path) {
            idx.put_note(st.clone(), String::from_utf8_lossy(data).into_owned(), h);
        } else {
            idx.put_entry(st.clone(), None);
        }
        changes.push(if existed { Change::Modified { entry: st.clone() } } else { Change::Created { entry: st.clone() } });
        Ok(WriteResult { entry: st, hash: index::hash_hex(&h), changes })
    }

    /// Create a folder and its parents if missing. Not an error if it exists.
    pub fn ensure_folder(&self, path: &str) -> Result<Vec<Change>> {
        let path = vpath::normalize(path)?;
        if path.is_empty() {
            return Ok(Vec::new());
        }
        let _g = self.op.lock();
        let mut idx = self.index.write();
        let mut changes = Vec::new();
        self.ensure_parents(&mut idx, &format!("{path}/x"), &mut changes)?;
        Ok(changes)
    }

    /// Remove `dir` and its parents while they are empty. Used after sync
    /// deletes the last file of a folder.
    pub fn prune_empty_folders(&self, dir: &str) -> Result<Vec<Change>> {
        let _g = self.op.lock();
        let mut changes = Vec::new();
        let mut d = vpath::normalize(dir)?;
        while !d.is_empty() && !vpath::is_hidden(&d) {
            if !self.fs.remove_empty_dir(&d)? {
                break;
            }
            self.index.write().remove(&d);
            changes.push(Change::Deleted { path: d.clone(), kind: EntryKind::Dir });
            d = vpath::parent(&d).to_string();
        }
        Ok(changes)
    }

    pub fn create_folder(&self, path: &str) -> Result<Vec<Change>> {
        let path = vpath::normalize(path)?;
        Self::check_new_path(&path)?;
        let _g = self.op.lock();
        if self.fs.stat(&path)?.is_some() {
            return Err(CoreError::AlreadyExists(path));
        }
        let mut idx = self.index.write();
        Self::check_case_twins(&idx, &path, None)?;
        self.check_new_folders(&idx, &path)?;
        let mut changes = Vec::new();
        self.ensure_parents(&mut idx, &path, &mut changes)?;
        self.fs.create_dir(&path)?;
        let st = self.fs.stat(&path)?.ok_or_else(|| CoreError::NotFound(path.clone()))?;
        idx.put_entry(st.clone(), None);
        changes.push(Change::Created { entry: st });
        Ok(changes)
    }

    /// Refuse to rename `from` to `to` when `to` differs only in case from
    /// another entry next to it, as `create_note` refuses a new one; `from`
    /// itself does not count, so `a.md` can still become `A.md`. For renames
    /// the user makes: `rename` does not check, since sync applies renames
    /// from other devices with it.
    pub fn check_rename(&self, from: &str, to: &str) -> Result<()> {
        let from = vpath::normalize(from)?;
        let to = vpath::normalize(to)?;
        // Not to a device name either, which `rename` takes from sync.
        let name = vpath::file_name(&to);
        if vpath::is_reserved_name(name) {
            return Err(CoreError::InvalidName(name.to_string()));
        }
        Self::check_case_twins(&self.index.read(), &to, Some(&from))
    }

    /// Rename or move a file or folder. The target folder must exist.
    pub fn rename(&self, from: &str, to: &str) -> Result<Vec<Change>> {
        let from = vpath::normalize(from)?;
        let to = vpath::normalize(to)?;
        if from.is_empty() {
            return Err(CoreError::InvalidPath(from));
        }
        if from == to {
            return Ok(Vec::new());
        }
        Self::check_moved_path(&to)?;
        if vpath::is_inside(&to, &from) {
            return Err(CoreError::MoveIntoSelf(from));
        }
        let _g = self.op.lock();
        let parent = vpath::parent(&to);
        if !parent.is_empty() && self.fs.stat(parent)?.is_none_or(|s| s.kind != EntryKind::Dir) {
            return Err(CoreError::NotFound(parent.to_string()));
        }
        self.fs.rename(&from, &to)?;
        let st = self.fs.stat(&to)?.ok_or_else(|| CoreError::NotFound(to.clone()))?;
        let mut idx = self.index.write();
        if idx.entry(&from).is_none() {
            // Not indexed yet (raced with an external create): index it now.
            self.load_into(&mut idx, st.clone());
            if st.kind == EntryKind::Dir {
                let (list, unreadable) = self.fs.list_partial(&to)?;
                for c in list {
                    self.load_into(&mut idx, c);
                }
                self.unreadable.lock().extend(unreadable);
            }
            return Ok(vec![Change::Created { entry: st }]);
        }
        idx.rename_tree(&from, &to, Some(st.mtime));
        // A file given a Markdown name (`todo.txt` -> `todo.md`) is a note
        // now. Rename keeps size and mtime, so no rescan would parse it.
        if st.kind == EntryKind::File && vpath::is_markdown(&to) && !vpath::is_markdown(&from) {
            self.load_into(&mut idx, st.clone());
        }
        Ok(vec![Change::Renamed { from, entry: st }])
    }

    /// Delete a file or folder (to the trash, depending on the fs settings).
    pub fn delete(&self, path: &str) -> Result<Vec<Change>> {
        let path = vpath::normalize(path)?;
        if path.is_empty() {
            return Err(CoreError::InvalidPath(path));
        }
        let _g = self.op.lock();
        let kind = self.fs.stat(&path)?.map(|s| s.kind).ok_or_else(|| CoreError::NotFound(path.clone()))?;
        self.fs.remove(&path)?;
        self.index.write().remove_tree(&path);
        Ok(vec![Change::Deleted { path, kind }])
    }

    /// Delete a file like `delete`, but only if its content still has the
    /// hash `base_hash`; otherwise `CoreError::Conflict`, as `write_file`.
    pub fn delete_file(&self, path: &str, base_hash: &str) -> Result<Vec<Change>> {
        let path = vpath::normalize(path)?;
        if path.is_empty() {
            return Err(CoreError::InvalidPath(path));
        }
        let base = parse_hash(base_hash).ok_or_else(|| CoreError::InvalidPath("bad hash".into()))?;
        let _g = self.op.lock();
        if index::hash_bytes(&self.fs.read(&path)?) != base {
            return Err(CoreError::Conflict(path));
        }
        self.fs.remove(&path)?;
        self.index.write().remove_tree(&path);
        Ok(vec![Change::Deleted { path, kind: EntryKind::File }])
    }

    /// First free path `dir/base.ext`, `dir/base 1.ext`, ...
    pub fn unique_path(&self, dir: &str, base: &str, ext: &str) -> Result<String> {
        let dir = vpath::normalize(dir)?;
        let idx = self.index.read();
        let ext = if ext.is_empty() { String::new() } else { format!(".{ext}") };
        for n in 0.. {
            let name = if n == 0 { format!("{base}{ext}") } else { format!("{base} {n}{ext}") };
            let p = vpath::join(&dir, &name);
            if idx.entry(&p).is_none() && idx.case_twin(&p, None).is_none() && self.fs.stat(&p)?.is_none() {
                return Ok(p);
            }
        }
        unreachable!()
    }

    /// Compare the whole vault with the index and apply the differences.
    pub fn rescan(&self) -> Result<Vec<Change>> {
        let _g = self.op.lock();
        let (disk, unreadable) = self.fs.list_partial("")?;
        let mut idx = self.index.write();
        let old = idx.entries().filter(|e| !unknown(&e.path, &unreadable, |p| disk.iter().any(|s| s.path == p))).cloned().collect();
        *self.unreadable.lock() = unreadable;
        Ok(self.apply_diff(&mut idx, old, disk))
    }

    /// Rescan only the given paths (and their children if they are folders).
    /// This is what the file watcher calls with the paths it saw change.
    pub fn rescan_paths(&self, hints: &[String]) -> Result<Vec<Change>> {
        let mut roots: BTreeSet<String> = BTreeSet::new();
        for h in hints {
            let Ok(p) = vpath::normalize(h) else { continue };
            if p.is_empty() {
                return self.rescan();
            }
            if !vpath::is_hidden(&p) {
                // The watcher watches a folder once, and can report a change
                // in it under the name of a symlink loop to it, which the
                // listing skips: rescan the folder's own path too.
                if self.fs.under_skipped_link(&p) {
                    roots.extend(self.fs.real_path(&p).filter(|q| !q.is_empty()));
                }
                roots.insert(p);
            }
        }
        // Drop roots that are inside another root.
        let outermost = |roots: &BTreeSet<String>| -> Vec<String> {
            roots.iter().filter(|p| !roots.iter().any(|q| vpath::is_inside(p, q))).cloned().collect()
        };
        // A hint whose folder is gone too is stale: the debouncer keeps one
        // queue per path, so an event for a file (a read, a save) queued just
        // before its folder was renamed can come in an earlier batch than the
        // rename. Rescan the nearest folder that still exists instead, so the
        // rename is seen whole and not as that file's deletion first. Only a
        // path that is really gone is lifted: one through a symlink loop, or
        // one that cannot be looked up, is handled below as it is.
        let gone = |p: &str| matches!(self.fs.stat(p), Ok(None));
        let mut lifted: BTreeSet<String> = BTreeSet::new();
        for r in outermost(&roots) {
            let mut target = r.as_str();
            if !self.fs.under_skipped_link(&r) && gone(&r) {
                let mut up = vpath::parent(&r);
                while !up.is_empty() && gone(up) {
                    up = vpath::parent(up);
                }
                if up != vpath::parent(&r) {
                    if up.is_empty() {
                        return self.rescan();
                    }
                    target = up;
                }
            }
            lifted.insert(target.to_string());
        }
        let roots = outermost(&lifted);
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let _g = self.op.lock();
        let mut idx = self.index.write();
        let mut old = Vec::new();
        let mut new = Vec::new();
        let mut seen_new: HashSet<String> = HashSet::new();
        let mut unreadable = Vec::new();
        for r in &roots {
            old.extend(idx.entries_under(r));
            // Through a symlink loop that the listing skips: not part of the
            // vault, though it leads somewhere on disk.
            if self.fs.under_skipped_link(r) {
                continue;
            }
            let st = match self.fs.stat(r) {
                Ok(st) => st,
                // Cannot be looked up (in a folder without permission): what
                // the index has there is unknown, not deleted.
                Err(e) => {
                    log::warn!("rescan: skipping {r:?}: {e}");
                    unreadable.push(r.clone());
                    continue;
                }
            };
            if let Some(st) = st {
                let is_dir = st.kind == EntryKind::Dir;
                // Make sure unindexed parent folders are picked up as well.
                let mut p = vpath::parent(r);
                while !p.is_empty() && idx.entry(p).is_none() {
                    if let Some(ps) = self.fs.stat(p)?
                        && seen_new.insert(ps.path.clone())
                    {
                        new.push(ps);
                    }
                    p = vpath::parent(p);
                }
                if seen_new.insert(st.path.clone()) {
                    new.push(st);
                }
                if is_dir {
                    let (list, skipped) = self.fs.list_partial(r)?;
                    for c in list {
                        if seen_new.insert(c.path.clone()) {
                            new.push(c);
                        }
                    }
                    unreadable.extend(skipped);
                }
            }
        }
        old.retain(|e| !unknown(&e.path, &unreadable, |p| seen_new.contains(p)));
        {
            let mut u = self.unreadable.lock();
            u.retain(|d| !roots.iter().any(|r| vpath::is_same_or_inside(d, r)));
            u.extend(unreadable);
        }
        Ok(self.apply_diff(&mut idx, old, new))
    }

    /// Core of change detection: turn "what the index has" vs "what is on
    /// disk" into changes, detecting renames, and update the index.
    fn apply_diff(&self, idx: &mut Index, old: Vec<FileStat>, new: Vec<FileStat>) -> Vec<Change> {
        let old_map: HashMap<String, FileStat> = old.into_iter().map(|s| (s.path.clone(), s)).collect();
        let new_map: HashMap<String, FileStat> = new.into_iter().map(|s| (s.path.clone(), s)).collect();

        let mut deleted: Vec<FileStat> = old_map
            .values()
            .filter(|o| new_map.get(&o.path).is_none_or(|n| n.kind != o.kind))
            .cloned()
            .collect();
        let mut created: Vec<FileStat> = new_map
            .values()
            .filter(|n| old_map.get(&n.path).is_none_or(|o| o.kind != n.kind))
            .cloned()
            .collect();
        let mut modified: Vec<FileStat> = new_map
            .values()
            .filter(|n| {
                n.kind == EntryKind::File
                    && old_map
                        .get(&n.path)
                        .is_some_and(|o| o.kind == n.kind && (o.size != n.size || o.mtime != n.mtime))
            })
            .cloned()
            .collect();
        deleted.sort_by(|a, b| a.path.cmp(&b.path));
        created.sort_by(|a, b| a.path.cmp(&b.path));
        modified.sort_by(|a, b| a.path.cmp(&b.path));

        let mut changes = Vec::new();

        // Folder renames: same set of (relative path, size) underneath.
        // Each entry adds itself to the signature of every changed folder
        // above it, in one pass, so replacing many folders stays linear.
        let signatures = |set: &[FileStat]| -> HashMap<String, Vec<(String, u64)>> {
            let mut sigs: HashMap<String, Vec<(String, u64)>> =
                set.iter().filter(|s| s.kind == EntryKind::Dir).map(|s| (s.path.clone(), Vec::new())).collect();
            for s in set {
                let mut dir = vpath::parent(&s.path);
                while !dir.is_empty() {
                    if let Some(sig) = sigs.get_mut(dir) {
                        sig.push((s.path[dir.len() + 1..].to_string(), s.size));
                    }
                    dir = vpath::parent(dir);
                }
            }
            sigs.values_mut().for_each(|sig| sig.sort());
            sigs
        };
        let del_sigs = signatures(&deleted);
        let mut cre_sigs = signatures(&created);
        let del_dirs: Vec<&FileStat> = deleted.iter().filter(|s| s.kind == EntryKind::Dir).collect();
        let cre_dir_count = cre_sigs.len();
        // Created folders by signature, in path order.
        let mut by_sig: HashMap<Vec<(String, u64)>, VecDeque<FileStat>> = HashMap::new();
        for c in created.iter().filter(|s| s.kind == EntryKind::Dir) {
            let sig = cre_sigs.remove(&c.path).unwrap_or_default();
            by_sig.entry(sig).or_default().push_back(c.clone());
        }
        let mut renamed_from: HashSet<String> = HashSet::new();
        let mut renamed_to: HashSet<String> = HashSet::new();
        let mut dir_renames: Vec<(String, FileStat)> = Vec::new();
        for d in &del_dirs {
            if same_or_inside_any(&d.path, &renamed_from) {
                continue;
            }
            let sig = &del_sigs[&d.path];
            if sig.is_empty() && !(del_dirs.len() == 1 && cre_dir_count == 1) {
                continue;
            }
            let Some(cands) = by_sig.get_mut(sig) else { continue };
            // A candidate inside a folder already renamed stays unusable.
            while let Some(c) = cands.pop_front() {
                if !same_or_inside_any(&c.path, &renamed_to) {
                    renamed_from.insert(d.path.clone());
                    renamed_to.insert(c.path.clone());
                    dir_renames.push((d.path.clone(), c));
                    break;
                }
            }
        }
        for (from, to) in &dir_renames {
            idx.rename_tree(from, &to.path, Some(to.mtime));
            changes.push(Change::Renamed { from: from.clone(), entry: to.clone() });
        }
        deleted.retain(|s| !same_or_inside_any(&s.path, &renamed_from));
        // Children that changed while moving are handled as modifications.
        // Notes are always read: the same names, sizes and times can be a
        // different folder (restored from an archive) with other text.
        let (moved, rest): (Vec<FileStat>, Vec<FileStat>) =
            created.into_iter().partition(|s| same_or_inside_any(&s.path, &renamed_to));
        created = rest;
        for m in moved {
            if renamed_to.contains(&m.path) {
                continue;
            }
            let note = m.kind == EntryKind::File && vpath::is_markdown(&m.path);
            if note || idx.entry(&m.path).is_some_and(|e| e.size != m.size || e.mtime != m.mtime) {
                modified.push(m);
            }
        }

        // Read and hash new and modified notes.
        let mut new_hashes: HashMap<String, (Hash, String)> = HashMap::new();
        for s in created.iter().chain(modified.iter()) {
            if s.kind == EntryKind::File
                && vpath::is_markdown(&s.path)
                && let Ok(bytes) = self.fs.read(&s.path)
            {
                let h = index::hash_bytes(&bytes);
                new_hashes.insert(s.path.clone(), (h, String::from_utf8_lossy(&bytes).into_owned()));
            }
        }

        // File renames: a deleted and a created file with identical content
        // (notes) or identical size+mtime+extension (attachments), paired by
        // `pair_renames`. An empty file says nothing about where it went.
        #[derive(PartialEq, Eq, Hash)]
        enum Key {
            Note(Hash),
            Attachment(Option<String>, u64, i64),
        }
        let key = |s: &FileStat, hash: Option<Hash>| -> Option<Key> {
            if s.kind != EntryKind::File || s.size == 0 {
                None
            } else if vpath::is_markdown(&s.path) {
                hash.map(Key::Note)
            } else {
                Some(Key::Attachment(vpath::extension(&s.path), s.size, s.mtime))
            }
        };
        let dels: Vec<(&str, Key)> =
            deleted.iter().filter_map(|s| Some((s.path.as_str(), key(s, idx.hashes.get(&s.path).copied())?))).collect();
        let (cres, cre_stats): (Vec<(&str, Key)>, Vec<&FileStat>) = created
            .iter()
            .filter_map(|s| Some(((s.path.as_str(), key(s, new_hashes.get(&s.path).map(|x| x.0))?), s)))
            .unzip();
        let file_renames: Vec<(String, FileStat)> = pair_renames(&dels, &cres)
            .into_iter()
            .map(|(d, c)| (dels[d].0.to_string(), cre_stats[c].clone()))
            .collect();
        for (from, to) in &file_renames {
            idx.rename_tree(from, &to.path, Some(to.mtime));
            changes.push(Change::Renamed { from: from.clone(), entry: to.clone() });
        }
        let renamed_files: HashSet<&str> = file_renames.iter().map(|(f, _)| f.as_str()).collect();
        deleted.retain(|s| !renamed_files.contains(s.path.as_str()));
        let renamed_files: HashSet<&str> = file_renames.iter().map(|(_, t)| t.path.as_str()).collect();
        created.retain(|s| !renamed_files.contains(s.path.as_str()));

        // Deletions: report only the top-most deleted entry.
        let del_paths: HashSet<&str> = deleted.iter().map(|s| s.path.as_str()).collect();
        for s in &deleted {
            if same_or_inside_any(vpath::parent(&s.path), &del_paths) {
                continue;
            }
            idx.remove_tree(&s.path);
            changes.push(Change::Deleted { path: s.path.clone(), kind: s.kind });
        }

        // Creations, parents first (sorted by path).
        for s in created {
            match new_hashes.remove(&s.path) {
                Some((h, text)) => idx.put_note(s.clone(), text, h),
                None => idx.put_entry(s.clone(), None),
            }
            changes.push(Change::Created { entry: s });
        }

        // Modifications. Same content (e.g. our own write, `touch`) is silent.
        for s in modified {
            if vpath::is_markdown(&s.path) {
                match new_hashes.remove(&s.path) {
                    // Same text: keep the parsed note, update size and time.
                    Some((h, _)) if idx.hashes.get(&s.path) == Some(&h) && idx.note(&s.path).is_some() => {
                        idx.put_entry(s, Some(h));
                    }
                    Some((h, text)) => {
                        let same = idx.hashes.get(&s.path) == Some(&h);
                        idx.put_note(s.clone(), text, h);
                        if !same {
                            changes.push(Change::Modified { entry: s });
                        }
                    }
                    None => idx.put_entry(s, None),
                }
            } else {
                idx.put_entry(s.clone(), None);
                changes.push(Change::Modified { entry: s });
            }
        }
        changes
    }

    fn config_path(name: &str) -> Result<String> {
        let rel = vpath::normalize(name)?;
        if rel.is_empty() || rel.split('/').any(|c| c.starts_with('.')) {
            return Err(CoreError::InvalidPath(name.to_string()));
        }
        Ok(format!("{CONFIG_DIR}/{rel}"))
    }

    /// Read `.cairn/<name>`; `None` if it does not exist.
    pub fn read_config(&self, name: &str) -> Result<Option<String>> {
        let p = Self::config_path(name)?;
        match self.fs.read(&p) {
            Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
            Err(CoreError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write `.cairn/<name>`, creating folders as needed. A symlink there is
    /// followed only if it leads to a file in the vault; a link out of the
    /// vault (one a received vault came with) is replaced by the file. If a
    /// folder on the way (`.cairn`, `.cairn/snippets`) leads out of the
    /// vault, nothing is written and the error names that folder.
    pub fn write_config(&self, name: &str, content: &str) -> Result<()> {
        let p = Self::config_path(name)?;
        let _g = self.op.lock();
        if let Some(dir) = self.fs.folder_outside(vpath::parent(&p)) {
            return Err(CoreError::Io(format!("The \"{dir}\" folder leads outside the notebook.")));
        }
        self.fs.create_dir(vpath::parent(&p))?;
        self.fs.write_in_vault(&p, content.as_bytes())?;
        Ok(())
    }

    /// Names of the files directly inside `.cairn/<dir>`.
    pub fn list_config(&self, dir: &str) -> Result<Vec<String>> {
        let p = Self::config_path(dir)?;
        let mut names: Vec<String> = self
            .fs
            .list_partial(&p)?
            .0
            .into_iter()
            .filter(|s| s.kind == EntryKind::File && vpath::parent(&s.path) == p)
            .map(|s| vpath::file_name(&s.path).to_string())
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        self.index.read().search(query, limit)
    }

    pub fn backlinks(&self, path: &str) -> Vec<Backlinks> {
        self.index.read().backlinks(path)
    }

    pub fn outgoing(&self, path: &str) -> Vec<OutgoingLink> {
        self.index.read().outgoing(path)
    }

    pub fn tags(&self) -> Vec<index::TagCount> {
        self.index.read().tags()
    }

    pub fn graph(&self, include_unresolved: bool) -> index::Graph {
        self.index.read().graph(include_unresolved)
    }

    pub fn note_info(&self, path: &str) -> Option<index::NoteInfo> {
        self.index.read().note_info(path)
    }

    pub fn resolve(&self, target: &str, source: &str) -> Option<String> {
        self.index.read().resolve(target, source)
    }

    /// What a click on a link of `kind` in `source` opens (see
    /// [`Index::resolve_target`]).
    pub fn resolve_target(&self, target: &str, kind: LinkKind, source: &str) -> Option<String> {
        self.index.read().resolve_target(target, kind, source)
    }
}
