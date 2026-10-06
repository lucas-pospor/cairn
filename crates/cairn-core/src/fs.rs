//! File system abstraction.
//!
//! Everything above this module talks to a [`VaultFs`], never to `std::fs`.
//! Desktop uses [`StdFs`]. Android can use `StdFs` on app storage, or a
//! Storage Access Framework implementation that only has `content://` URIs.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use unicode_normalization::{is_nfc, UnicodeNormalization};

use crate::error::{CoreError, Result};
use crate::path as vpath;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
}

/// One file or folder as seen on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStat {
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Modification time, milliseconds since the Unix epoch.
    pub mtime: i64,
}

/// What `remove` does with the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TrashMode {
    /// OS trash / recycle bin, falling back to the vault trash.
    #[default]
    System,
    /// `<vault>/.trash/`.
    Vault,
    /// Delete for good.
    Permanent,
}

pub trait VaultFs: Send + Sync {
    /// Recursively list everything under folder `dir` ("" = whole vault),
    /// excluding hidden entries. `dir` itself is not included. Symlinks are
    /// followed, but a folder that leads back to one the listing is in (a
    /// loop: `loop -> .`, a link to a parent, a bind mount of one) is
    /// skipped, and no operation goes through it. Any other folder link is
    /// listed, also one that shows a folder a second time (see
    /// `other_names`).
    fn list(&self, dir: &str) -> Result<Vec<FileStat>>;
    /// Like `list`, but a folder that cannot be read (no permission), or an
    /// entry that cannot be looked up, is skipped instead of failing the
    /// listing, and returned in the second list: what is there is unknown,
    /// not deleted. Only an unreadable vault root is an error. The default
    /// is `list`, which reads every folder or fails.
    fn list_partial(&self, dir: &str) -> Result<(Vec<FileStat>, Vec<String>)> {
        Ok((self.list(dir)?, Vec::new()))
    }
    /// True if `path` is, or runs through, a loop that `list` skips. Every
    /// operation on such a path fails instead of acting through the link.
    fn under_skipped_link(&self, _path: &str) -> bool {
        false
    }
    /// For a path that runs through a loop `list` skips, the vault path of
    /// the same entry, with the loop taken out (`loop/a/n.md` is `a/n.md`
    /// for `loop -> .`). `None` for a path without a loop, or for a loop
    /// itself.
    fn real_path(&self, _path: &str) -> Option<String> {
        None
    }
    /// The other vault paths under which the last listing of the whole
    /// vault found the file at `path`, less any that leads somewhere else
    /// now: one file on disk reached under more than one name, through a
    /// folder that is linked in twice (`alias -> notes`, two links to one
    /// folder outside the vault, a bind mount) or a symlink to another
    /// note. A hard link is a file of its own (a change through one name
    /// never deletes or moves the other, and a write keeps both). Sorted;
    /// empty for a file found under one path. Sync syncs one of the names
    /// (FINDING-224). The default is none.
    fn other_names(&self, _path: &str) -> Vec<String> {
        Vec::new()
    }
    /// True if `path` is a symlink, or a symlink is on the way to it: the
    /// file is not where the path says. Of the names in `other_names`, sync
    /// prefers one without. The default is false.
    fn via_link(&self, _path: &str) -> bool {
        false
    }
    /// True if vault paths `a` and `b` are one file on disk now, as for
    /// `other_names`, also for a name that no listing has seen yet. The
    /// default is false.
    fn same_file(&self, _a: &str, _b: &str) -> bool {
        false
    }
    /// True if `path` really is outside the vault's notes on disk: a symlink
    /// on the way to it, or to the part of it that exists, leads out of the
    /// vault, into what the app keeps out of it (`.git`, `.cairn`, `.trash`),
    /// or from a Markdown name to a file that is not Markdown; or a link
    /// cannot be followed. Vault paths have no "..", but symlinks are
    /// followed everywhere else.
    fn leads_outside(&self, _path: &str) -> bool {
        false
    }
    /// `None` if the path does not exist.
    fn stat(&self, path: &str) -> Result<Option<FileStat>>;
    fn read(&self, path: &str) -> Result<Vec<u8>>;
    /// Create or replace a file. Implementations should make this atomic.
    /// The parent folder must exist.
    fn write(&self, path: &str, data: &[u8]) -> Result<FileStat>;
    /// Like `write`, but a symlink at `path` is followed only if the file it
    /// leads to is in the vault: a link out of it is replaced by the new
    /// file, and the file it led to is left alone. For the files the app
    /// keeps for itself (`.cairn/`), which a vault from elsewhere can link
    /// to any file. The default is `write`.
    fn write_in_vault(&self, path: &str, data: &[u8]) -> Result<FileStat> {
        self.write(path, data)
    }
    /// The first folder on the way to `dir` (`dir` included) that is not
    /// really in the vault folder: a symlink leads it out, or cannot be
    /// followed. `None` if every one of them that exists is in the vault (a
    /// missing one would be made there). For the folders the app keeps for
    /// itself (`.cairn/`), which a vault from elsewhere can link anywhere.
    /// The default is `None`.
    fn folder_outside(&self, _dir: &str) -> Option<String> {
        None
    }
    /// True if this file system ignores case and refuses a name that differs
    /// only in case from another entry in its folder, as Android shared
    /// storage does through SAF. Sync then leaves a remote file under such a
    /// name out and lists it as not synced, instead of giving it a conflict
    /// copy name, which every device would take. The default is false.
    fn refuses_case_twins(&self) -> bool {
        false
    }
    /// Replace existing file `path` with `data`, but only if its current
    /// content passes `check`; otherwise, or if the file is gone,
    /// `CoreError::Conflict`. Implementations should also return `Conflict`
    /// if the file changes while `data` is being written. The default
    /// checks, then writes.
    fn write_if(&self, path: &str, data: &[u8], check: &dyn Fn(&[u8]) -> bool) -> Result<FileStat> {
        match self.read(path) {
            Ok(cur) if check(&cur) => self.write(path, data),
            Ok(_) | Err(CoreError::NotFound(_)) => Err(CoreError::Conflict(path.to_string())),
            Err(e) => Err(e),
        }
    }
    /// Create a folder and any missing parents.
    fn create_dir(&self, path: &str) -> Result<()>;
    /// Move a file or folder. Fails if `to` exists (a case-only rename of
    /// the same entry is allowed).
    fn rename(&self, from: &str, to: &str) -> Result<()>;
    /// Delete a file or folder (recursively), honoring the trash mode.
    fn remove(&self, path: &str) -> Result<()>;
    /// Remove a folder only if it is empty (never goes to the trash).
    fn remove_empty_dir(&self, path: &str) -> Result<bool>;
    /// Human-readable location of the vault.
    fn describe(&self) -> String;
    /// Where `path` is on the local file system, when the vault is a
    /// folder there.
    fn os_path(&self, _path: &str) -> Option<PathBuf> {
        None
    }
    /// Files and folders that listings skip because their name has a
    /// backslash, an ordinary character in Linux and Android names but a
    /// separator in vault paths, so no vault path can reach them. Each is
    /// its path on disk relative to the vault folder (`notes/a\b.md`), as
    /// the last listing of its folder found it, if it is still there;
    /// hidden folders are left out. Sorted.
    fn skipped_backslash_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// A number that changes whenever the file at `path` is written or
    /// replaced, even when its size and mtime stay the same (a tool that
    /// puts the old mtime back, another file moved over it). `None` when
    /// the file system has nothing like it, or the file cannot be looked at.
    fn change_stamp(&self, _path: &str) -> Option<u64> {
        None
    }
}

/// `std::fs`-backed vault.
pub struct StdFs {
    root: PathBuf,
    trash: TrashMode,
    /// What the last listing of each folder (on-disk path) found that a
    /// lookup by vault name cannot find by itself.
    listed: RwLock<HashMap<PathBuf, Listed>>,
    /// The other names of each file that the last listing of the whole
    /// vault found under more than one vault path (see `other_names`).
    names: RwLock<HashMap<String, Vec<String>>>,
}

/// Names in one folder that a lookup by vault name cannot find by itself.
#[derive(Default)]
struct Listed {
    /// On-disk name by vault name, for names that are neither their vault
    /// name nor its NFD form: other Unicode forms (Kelvin sign, CJK
    /// compatibility ideographs, mixed forms), and twins.
    odd: HashMap<String, OsString>,
    /// Twin name by on-disk name. Each of these is only found by its twin
    /// name, not by its own vault name.
    twins: HashMap<OsString, String>,
    /// On-disk names with a backslash, which the listing skipped.
    backslash: Vec<OsString>,
}

/// Vault name of an on-disk file name: its NFC form. `None` for a name
/// with a backslash, which is a separator in vault paths but an ordinary
/// character in Linux names, so no vault path can reach the file.
fn vault_name(name: &str) -> Option<Cow<'_, str>> {
    if name.contains('\\') {
        None
    } else if is_nfc(name) {
        Some(Cow::Borrowed(name))
    } else {
        Some(Cow::Owned(name.nfc().collect()))
    }
}

/// A free name for a twin of vault name `name`: "café (Unicode twin).md",
/// then "café (Unicode twin 2).md", ... Names in `taken` (lowercase, so
/// that no twin name only differs in case from another name) are skipped;
/// the one returned is added.
fn twin_name(name: &str, taken: &mut HashSet<String>) -> String {
    let (stem, ext) = split_ext(name);
    let mut n = 1;
    loop {
        let num = if n == 1 { String::new() } else { format!(" {n}") };
        let twin = format!("{stem} (Unicode twin{num}){ext}");
        if taken.insert(twin.to_lowercase()) {
            return twin;
        }
        n += 1;
    }
}

/// How on-disk name `os_name` is spelled compared with its vault name
/// `name`: 0 the same, 1 its NFD form, 2 another form. Of several names for
/// one vault name, the lowest rank is used.
fn form_rank(name: &str, os_name: &OsStr) -> u8 {
    if os_name == OsStr::new(name) {
        0
    } else if os_name.to_str().is_some_and(|n| n.chars().eq(name.nfd())) {
        1
    } else {
        2
    }
}

/// True if vault name `name` can stand for an on-disk name other than
/// itself. Plain ASCII has one form only, except for three characters that
/// are also the NFC of a non-ASCII one (Kelvin sign, Greek question mark,
/// Greek varia). A twin name keeps the characters of the name it is a twin
/// of.
fn has_other_forms(name: &str) -> bool {
    !name.is_ascii() || name.contains(['K', ';', '`'])
}

/// Identity of a folder on disk, to tell a folder that leads back to one it
/// is in.
#[cfg(unix)]
type DirId = (u64, u64);
#[cfg(not(unix))]
type DirId = PathBuf;

#[cfg(unix)]
fn dir_id(_abs: &Path, md: &fs::Metadata) -> Option<DirId> {
    use std::os::unix::fs::MetadataExt;
    Some((md.dev(), md.ino()))
}

/// Without a file id in stable std, the canonical path.
#[cfg(not(unix))]
fn dir_id(abs: &Path, _md: &fs::Metadata) -> Option<DirId> {
    abs.canonicalize().ok()
}

/// Where a file is on disk: the identity of the folder it really is in,
/// and its name there. Every vault path that reaches the file has it: a
/// folder reached through two links, a bind mount, a symlink to the file.
/// A hard link is an entry of its own, with a location of its own.
type FileId = (DirId, OsString);

/// The location of the file at `abs`, with every link on the way and the
/// file's own link followed.
fn location_of(abs: &Path) -> Option<FileId> {
    let real = abs.canonicalize().ok()?;
    let dir = real.parent()?;
    let md = fs::metadata(dir).ok()?;
    Some((dir_id(dir, &md)?, real.file_name()?.to_os_string()))
}

/// For each file of listing `out` that is one file on disk with others,
/// the paths of the others, sorted. `ids` has the id and the index in
/// `out` of each file.
fn other_names_of(mut ids: Vec<(FileId, usize)>, out: &[FileStat]) -> HashMap<String, Vec<String>> {
    ids.sort_unstable();
    let mut names = HashMap::new();
    for same in ids.chunk_by(|a, b| a.0 == b.0).filter(|s| s.len() > 1) {
        let mut paths: Vec<&str> = same.iter().map(|(_, i)| out[*i].path.as_str()).collect();
        paths.sort_unstable();
        for p in &paths {
            names.insert(p.to_string(), paths.iter().filter(|q| *q != p).map(|q| q.to_string()).collect());
        }
    }
    names
}

/// What a partial listing could not read (`unreadable`) is unknown, not
/// gone: the other names that the listing before it (`old`) found there
/// still count, as for a link in a folder that lost its read permission.
/// They are added to `names`, the other names this listing found.
fn keep_unknown(names: &mut HashMap<String, Vec<String>>, old: &HashMap<String, Vec<String>>, unreadable: &[String]) {
    if unreadable.is_empty() {
        return;
    }
    let unknown = |p: &str| unreadable.iter().any(|u| vpath::is_same_or_inside(p, u));
    for (p, others) in old {
        let kept: Vec<&String> = others.iter().filter(|o| unknown(p) || unknown(o)).collect();
        if !kept.is_empty() {
            let list = names.entry(p.clone()).or_default();
            list.extend(kept.into_iter().cloned());
            list.sort_unstable();
            list.dedup();
        }
    }
}

/// State of one listing.
struct Walk {
    /// The folders being listed, from the vault root down.
    ancestors: Vec<DirId>,
    /// Folders and entries that could not be read, when the caller takes a
    /// partial listing (`None`: an unreadable folder fails the listing).
    unreadable: Option<Vec<String>>,
    /// The id of each file listed and its index in the listing, when the
    /// whole vault is listed (`None` otherwise), to find the files it
    /// reaches under more than one vault path.
    files: Option<Vec<(FileId, usize)>>,
}

/// A vault path followed on disk, see `StdFs::follow`.
struct Way {
    /// The on-disk path, as `StdFs::abs` gives it.
    abs: PathBuf,
    /// The folders on the way, the vault root first, up to the first name
    /// that is not a folder (loops taken out).
    dirs: Vec<DirId>,
    /// True if the path is, or runs through, a loop.
    looped: bool,
    /// True if the last name is a loop.
    ends_at_loop: bool,
    /// The vault path with every loop taken out.
    real: String,
}

/// The error for an operation on a path that runs through a loop.
fn loop_error(path: &str) -> CoreError {
    CoreError::InvalidPath(format!("{path} goes through a folder symlink that loops back"))
}

impl StdFs {
    pub fn new(root: impl Into<PathBuf>, trash: TrashMode) -> Result<Self> {
        let root: PathBuf = root.into();
        let root = root
            .canonicalize()
            .map_err(|e| CoreError::io(&root.to_string_lossy(), e))?;
        if !root.is_dir() {
            return Err(CoreError::io(&root.to_string_lossy(), std::io::ErrorKind::NotADirectory.into()));
        }
        Ok(StdFs { root, trash, listed: RwLock::new(HashMap::new()), names: RwLock::new(HashMap::new()) })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// On-disk path of vault path `p`.
    ///
    /// Vault paths are NFC, but a name made outside Cairn can be in another
    /// Unicode form on disk (macOS writes NFD; the Kelvin and Ohm signs and
    /// CJK compatibility ideographs change under NFC), or be listed under a
    /// twin name. A component missing under its vault name is looked for
    /// under its other forms, without reading the folder. A component
    /// missing in every form (a new file) keeps its vault name.
    fn abs(&self, p: &str) -> PathBuf {
        let parts: Vec<&str> = p.split('/').filter(|c| !c.is_empty()).collect();
        let mut out = self.root.clone();
        out.extend(&parts);
        if !parts.iter().any(|c| has_other_forms(c)) || fs::symlink_metadata(&out).is_ok() {
            return out;
        }
        let mut out = self.root.clone();
        for (i, c) in parts.iter().enumerate() {
            if !has_other_forms(c) || fs::symlink_metadata(out.join(c)).is_ok() {
                out.push(c);
            } else if let Some(name) = self.other_form(&out, c) {
                out.push(name);
            } else {
                out.extend(&parts[i..]);
                break;
            }
        }
        out
    }

    /// The name in folder `dir` (on disk) that vault name `name` stands for,
    /// in the order `list` prefers: the NFD form (macOS), else the name the
    /// last listing of `dir` used. A name that listing showed under a twin
    /// name is not used.
    fn other_form(&self, dir: &Path, name: &str) -> Option<OsString> {
        let listed = self.listed.read();
        let listed = listed.get(dir);
        let nfd: String = name.nfd().collect();
        let twin = listed.is_some_and(|l| l.twins.contains_key(OsStr::new(&nfd)));
        if nfd != name && !twin && fs::symlink_metadata(dir.join(&nfd)).is_ok() {
            return Some(nfd.into());
        }
        let found = listed?.odd.get(name)?;
        fs::symlink_metadata(dir.join(found)).is_ok().then(|| found.clone())
    }

    /// Convert an absolute OS path (e.g. from the watcher) to a vault path.
    /// A name that is, or may now be, listed under a twin name gives the
    /// path of its folder instead: a rescan of the folder lists it under the
    /// right name.
    pub fn to_vault_path(&self, abs: &Path) -> Option<String> {
        let rel = abs.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        let mut dir = self.root.clone();
        for c in rel.components() {
            let Component::Normal(os_name) = c else { return None };
            let name = vault_name(os_name.to_str()?)?;
            if self.may_be_twin(&dir, os_name, &name) {
                break;
            }
            parts.push(name.into_owned());
            dir.push(os_name);
        }
        Some(parts.join("/"))
    }

    /// True if on-disk name `os_name` in folder `dir` (on disk), whose vault
    /// name is `name`, is or may be listed under a twin name: the last
    /// listing did so, or a name `list` prefers for `name` (the name
    /// itself, its NFD form) is there too. Checked without reading the
    /// folder.
    fn may_be_twin(&self, dir: &Path, os_name: &OsStr, name: &str) -> bool {
        if os_name == OsStr::new(name) {
            return false; // the name itself always comes first
        }
        if self.listed.read().get(dir).is_some_and(|l| l.twins.contains_key(os_name)) {
            return true;
        }
        let there = |n: &str| fs::symlink_metadata(dir.join(n)).is_ok();
        there(name) || form_rank(name, os_name) == 2 && there(&name.nfd().collect::<String>())
    }

    fn stat_abs(&self, path: &str, abs: &Path) -> Result<Option<FileStat>> {
        // Follow symlinks for files and folders so linked folders still show
        // (callers keep loops out).
        let md = match fs::metadata(abs) {
            Ok(m) => m,
            Err(e) if is_gone(&e) => return Ok(None),
            Err(e) => return Err(CoreError::io(path, e)),
        };
        Ok(Some(stat_from_meta(path.to_string(), &md)))
    }

    /// Follow vault path `path` on disk from the vault root, one name at a
    /// time, as `list` does. A folder that leads back to one already on the
    /// way is a loop, which `list` skips: the way goes on from the folder
    /// it leads to.
    fn follow(&self, path: &str) -> Way {
        let abs = self.abs(path);
        let names: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
        let disk: Vec<&OsStr> = abs.strip_prefix(&self.root).map(|r| r.iter().collect()).unwrap_or_default();
        let id = |p: &Path| fs::metadata(p).ok().filter(|md| md.is_dir()).and_then(|md| dir_id(p, &md));
        // (identity, on-disk path) of each folder on the way, and the vault
        // names that lead to them.
        let mut dirs = vec![(id(&self.root), self.root.clone())];
        let mut real: Vec<&str> = Vec::new();
        let (mut looped, mut ends_at_loop, mut rest) = (false, false, names.len());
        for (i, (name, os)) in names.iter().zip(&disk).enumerate() {
            let p = dirs[dirs.len() - 1].1.join(os);
            let Some(id) = id(&p) else {
                (rest, ends_at_loop) = (i, false); // a file, or nothing yet
                break;
            };
            ends_at_loop = match dirs.iter().position(|d| d.0.as_ref() == Some(&id)) {
                Some(j) => {
                    dirs.truncate(j + 1);
                    real.truncate(j);
                    true
                }
                None => {
                    dirs.push((Some(id), p));
                    real.push(name);
                    false
                }
            };
            looped |= ends_at_loop;
        }
        real.extend(&names[rest..]);
        Way { abs, dirs: dirs.into_iter().filter_map(|d| d.0).collect(), looped, ends_at_loop, real: real.join("/") }
    }

    /// On-disk path of vault path `path` for an operation, which must not
    /// act through a loop that `list` skips.
    fn op_abs(&self, path: &str) -> Result<PathBuf> {
        let way = self.follow(path);
        if way.looped {
            return Err(loop_error(path));
        }
        Ok(way.abs)
    }

    /// List folder `dir` ("" = whole vault) recursively.
    fn walk(&self, dir: &str, unreadable: Option<Vec<String>>) -> Result<(Vec<FileStat>, Vec<String>)> {
        // The folders on the way to `dir` count as entered: a link below it
        // back to one of them is a loop.
        let way = self.follow(dir);
        if way.looped {
            return Err(loop_error(dir));
        }
        // A listing of the whole vault also finds the files it reaches
        // under more than one vault path.
        let mut walk = Walk { ancestors: way.dirs, unreadable, files: dir.is_empty().then(Vec::new) };
        let mut out = Vec::new();
        self.list_into(dir, &way.abs, 0, &mut walk, &mut out)?;
        if let Some(files) = walk.files {
            let mut names = other_names_of(files, &out);
            let unreadable = walk.unreadable.as_deref().unwrap_or_default();
            let mut known = self.names.write();
            keep_unknown(&mut names, &known, unreadable);
            *known = names;
        }
        Ok((out, walk.unreadable.unwrap_or_default()))
    }

    /// The location of the file at vault path `path` now, as a listing
    /// finds it: `Ok(None)` if there is no file there, `Err` if it cannot
    /// be looked up (no permission).
    fn id_now(&self, path: &str) -> std::result::Result<Option<FileId>, ()> {
        let Ok(abs) = self.op_abs(path) else { return Ok(None) };
        match fs::metadata(&abs) {
            Ok(md) if md.is_dir() => Ok(None),
            Ok(_) => Ok(location_of(&abs)),
            Err(e) if is_gone(&e) => Ok(None),
            Err(_) => Err(()),
        }
    }

    /// What to do when folder (or entry) `dir` cannot be read: a folder
    /// that is gone, or became a file since it was listed (git checkout,
    /// sync tools), has nothing to list. Any other error fails the listing,
    /// unless the caller takes a partial one. The vault root must always be
    /// readable.
    fn read_failed(&self, dir: &str, e: std::io::Error, walk: &mut Walk) -> Result<()> {
        if dir.is_empty() {
            return Err(CoreError::io(&self.root.to_string_lossy(), e));
        }
        if is_gone(&e) {
            return Ok(());
        }
        match &mut walk.unreadable {
            Some(skipped) => {
                log::warn!("skipping {dir:?}: {e}");
                skipped.push(dir.to_string());
                Ok(())
            }
            None => Err(CoreError::io(dir, e)),
        }
    }

    /// List folder `dir`, which is at `abs` on disk.
    fn list_into(&self, dir: &str, abs: &Path, depth: usize, walk: &mut Walk, out: &mut Vec<FileStat>) -> Result<()> {
        if depth > 64 {
            return Ok(()); // absurd nesting
        }
        let rd = match fs::read_dir(abs) {
            Ok(rd) => rd,
            Err(e) => return self.read_failed(dir, e, walk),
        };
        let start = out.len();
        let mut listed = Listed::default();
        // (vault name, form rank, on-disk name) of each visible entry
        let mut names: Vec<(String, u8, OsString)> = Vec::new();
        // Entries that are links themselves: a link to a file is found
        // where that file is.
        let mut links: HashSet<OsString> = HashSet::new();
        for ent in rd {
            let ent = match ent {
                Ok(ent) => ent,
                Err(e) => return self.read_failed(dir, e, walk),
            };
            let os_name = ent.file_name();
            let Some(name) = os_name.to_str() else {
                log::warn!("skipping non-UTF-8 file name in {dir:?}");
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Some(name) = vault_name(name) else {
                log::warn!("skipping {os_name:?} in {dir:?}: Cairn cannot use a backslash in a name");
                if !vpath::is_hidden(dir) {
                    listed.backslash.push(os_name);
                }
                continue;
            };
            if walk.files.is_some() && ent.file_type().is_ok_and(|t| t.is_symlink()) {
                links.insert(os_name.clone());
            }
            let rank = form_rank(&name, &os_name);
            names.push((name.into_owned(), rank, os_name));
        }
        // Names that differ only in Unicode form (café.md made on Linux and
        // on a Mac) have one vault name. The name `abs` reaches has it: the
        // name itself, else its NFD form, else the smallest. Each other one,
        // a twin, is listed under a twin name made from it, so that every
        // file shows and none is merged.
        names.sort();
        if names.windows(2).any(|w| w[0].0 == w[1].0) {
            let mut taken: HashSet<String> = names.iter().map(|n| n.0.to_lowercase()).collect();
            let mut first = 0;
            for i in 1..names.len() {
                if names[i].0 != names[first].0 {
                    first = i;
                    continue;
                }
                let twin = twin_name(&names[i].0, &mut taken);
                log::info!("listing {:?} in {dir:?} as {twin:?}: {:?} is the same name in another form", names[i].2, names[first].2);
                listed.twins.insert(names[i].2.clone(), twin.clone());
                // Neither the twin name nor its NFD form is on disk.
                (names[i].0, names[i].1) = (twin, 2);
            }
        }
        for (name, _, os_name) in names.iter().filter(|n| n.1 == 2) {
            listed.odd.insert(name.clone(), os_name.clone());
        }
        if !listed.odd.is_empty() || !listed.backslash.is_empty() {
            self.listed.write().insert(abs.to_path_buf(), listed);
        } else if !self.listed.read().is_empty() {
            self.listed.write().remove(abs);
        }
        // The folder's own identity, for the locations of its files.
        let here = walk.files.as_ref().and_then(|_| fs::metadata(abs).ok().and_then(|md| dir_id(abs, &md)));
        for (name, _, os_name) in &names {
            let path = vpath::join(dir, name);
            let ent_abs = abs.join(os_name);
            let md = match fs::metadata(&ent_abs) {
                Ok(m) => m,
                Err(e) if is_denied(&e) => {
                    // Without search (x) permission on the folder, none of
                    // its entries can be looked up: it cannot be read.
                    if fs::metadata(abs.join(".")).is_err_and(|e| is_denied(&e)) {
                        out.truncate(start);
                        if let Some(files) = &mut walk.files {
                            files.retain(|(_, i)| *i < start);
                        }
                        return self.read_failed(dir, e, walk);
                    }
                    // Only this entry cannot (a link into a folder this user
                    // may not enter, another user's mount): unknown, not gone.
                    self.read_failed(&path, e, walk)?;
                    continue;
                }
                Err(_) => continue, // broken symlink, raced delete
            };
            // A folder that leads back to one the listing is in (`loop -> .`,
            // a link to a parent, a bind mount of one) would list the same
            // notes again and again. Any other folder is listed, also when
            // a link shows it a second time.
            let id = if md.is_dir() { dir_id(&ent_abs, &md) } else { None };
            if id.as_ref().is_some_and(|id| walk.ancestors.contains(id)) {
                log::warn!("skipping {path:?}: it leads back to a folder it is in");
                continue;
            }
            let st = stat_from_meta(path.clone(), &md);
            let is_dir = st.kind == EntryKind::Dir;
            if !is_dir && let Some(files) = &mut walk.files {
                // A folder id is `Copy` on Unix only.
                #[allow(clippy::clone_on_copy)]
                let id = if links.contains(os_name) { location_of(&ent_abs) } else { here.clone().map(|d| (d, os_name.clone())) };
                files.extend(id.map(|id| (id, out.len())));
            }
            out.push(st);
            if is_dir {
                let n = walk.ancestors.len();
                walk.ancestors.extend(id);
                self.list_into(&path, &ent_abs, depth + 1, walk, out)?;
                walk.ancestors.truncate(n);
            }
        }
        Ok(())
    }

    fn move_to_vault_trash(&self, path: &str) -> Result<()> {
        let trash_dir = self.root.join(".trash");
        fs::create_dir_all(&trash_dir).map_err(|e| CoreError::io(".trash", e))?;
        let name = vpath::file_name(path);
        let mut target = trash_dir.join(name);
        let mut n = 1;
        while target.exists() {
            let (stem, ext) = split_ext(name);
            target = trash_dir.join(format!("{stem} {n}{ext}"));
            n += 1;
        }
        fs::rename(self.abs(path), target).map_err(|e| CoreError::io(path, e))
    }

    /// Where a write to `abs` goes. A symlinked note (dotfile managers,
    /// notes shared with a repo) is written where it leads, so the link
    /// stays a link; without `out`, only if that is in the vault. A link
    /// that is not followed (dangling, or out of the vault) is replaced.
    fn write_target(&self, abs: PathBuf, out: bool) -> PathBuf {
        if fs::symlink_metadata(&abs).is_ok_and(|md| md.file_type().is_symlink())
            && let Ok(target) = abs.canonicalize()
            && (out || target.starts_with(&self.root))
        {
            return target;
        }
        abs
    }

    /// `write`, following a symlink at `path` out of the vault only if `out`.
    fn write_following(&self, path: &str, data: &[u8], out: bool) -> Result<FileStat> {
        let abs = self.write_target(self.op_abs(path)?, out);
        // The file this replaces: none where a link is replaced, so nothing
        // of the file it leads to (mode, hard links) carries over.
        let old = fs::symlink_metadata(&abs).ok().filter(|md| !md.file_type().is_symlink());
        self.write_abs(path, &abs, data, old.as_ref(), false)
    }

    /// Write `data` to `abs` (vault path `path`, links resolved by
    /// `write_target`), replacing `old`, the file there now if any. With
    /// `checked`, `old` is the file the caller checked: unless it is still
    /// exactly that right before it is replaced, nothing changes and the
    /// result is `Conflict`.
    fn write_abs(&self, path: &str, abs: &Path, data: &[u8], old: Option<&fs::Metadata>, checked: bool) -> Result<FileStat> {
        let dir = abs.parent().ok_or_else(|| CoreError::InvalidPath(path.into()))?;
        if !dir.is_dir() {
            return Err(CoreError::NotFound(vpath::parent(path).to_string()));
        }
        // The rename below only needs write access to the folder, so check
        // the file's own mode: a read-only file is not replaced.
        if old.is_some_and(|md| md.is_file() && md.permissions().readonly()) {
            return Err(CoreError::Io(format!("\"{path}\" is read-only.")));
        }
        // Write to a hidden temp file next to the target, then rename over it,
        // so readers never see a half-written note.
        let (tmp, mut f) = create_temp(dir, old).map_err(|e| CoreError::io(path, e))?;
        let res = (|| -> std::io::Result<bool> {
            if let Some(old) = old {
                // Keep the original permissions (e.g. read-only for others),
                // set before any of the new content is in the file.
                let _ = f.set_permissions(old.permissions());
            }
            f.write_all(data)?;
            f.sync_all()?;
            drop(f);
            // Writing takes a while: another program's save that landed
            // since the check wins. Checked as late as possible.
            if checked && !old.is_some_and(|md| unchanged(md, abs)) {
                return Ok(false);
            }
            // A rename would split a hard-linked file from its other names,
            // so it is written in place. Should that fail, the complete
            // copy replaces it after all.
            if old.is_some_and(|md| hard_links(md) > 1) && write_in_place(abs, data).is_ok() {
                let _ = fs::remove_file(&tmp);
                return Ok(true);
            }
            fs::rename(&tmp, abs).map(|()| true)
        })();
        match res {
            Ok(true) => {}
            Ok(false) => {
                let _ = fs::remove_file(&tmp);
                return Err(CoreError::Conflict(path.to_string()));
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(CoreError::io(path, e));
            }
        }
        self.stat_abs(path, abs)?
            .ok_or_else(|| CoreError::NotFound(path.to_string()))
    }
}

/// True if `abs` is still the file `old` describes and nothing wrote to it:
/// the same inode with the same size, modification and change time. The
/// change time is set by the kernel on every write, even one that restores
/// the mtime.
#[cfg(unix)]
fn unchanged(old: &fs::Metadata, abs: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match fs::metadata(abs) {
        Ok(md) => {
            (md.dev(), md.ino(), md.size()) == (old.dev(), old.ino(), old.size())
                && (md.mtime(), md.mtime_nsec(), md.ctime(), md.ctime_nsec()) == (old.mtime(), old.mtime_nsec(), old.ctime(), old.ctime_nsec())
        }
        Err(_) => false,
    }
}

/// True if `abs` still has the size and modification time of `old`.
#[cfg(not(unix))]
fn unchanged(old: &fs::Metadata, abs: &Path) -> bool {
    match fs::metadata(abs) {
        Ok(md) => md.len() == old.len() && md.modified().ok() == old.modified().ok(),
        Err(_) => false,
    }
}

/// How many names (hard links) the file has.
#[cfg(unix)]
fn hard_links(md: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    md.nlink()
}

/// Without a link count in stable std, one.
#[cfg(not(unix))]
fn hard_links(_md: &fs::Metadata) -> u64 {
    1
}

/// Replace the content of existing file `abs` with `data`, keeping the file
/// itself (and so every hard link to it).
fn write_in_place(abs: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).open(abs)?;
    f.write_all(data)?;
    f.set_len(data.len() as u64)?;
    f.sync_all()
}

/// True if both paths are the same directory entry (not following symlinks).
#[cfg(unix)]
fn same_entry(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// True if both paths are the same directory entry. Without a file id in
/// stable std, compare the canonical paths, which carry the on-disk case.
#[cfg(not(unix))]
fn same_entry(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// True if `e` means the path does not exist, including a path below
/// something that is now a file (a folder replaced by a file).
fn is_gone(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory)
}

fn is_denied(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied
}

/// Create a new, empty temp file in folder `dir` for a write. The name is
/// short and random, not derived from the target, so it fits wherever the
/// target's name does and cannot be guessed; each call gets its own file.
fn create_temp(dir: &Path, old: Option<&fs::Metadata>) -> std::io::Result<(PathBuf, fs::File)> {
    use std::hash::{BuildHasher, Hasher};
    let names = std::iter::repeat_with(|| {
        // RandomState is seeded from the OS and has new keys on every call.
        let n = std::collections::hash_map::RandomState::new().build_hasher().finish();
        format!(".cairn-tmp-{n:016x}")
    });
    create_new_file(dir, old, names.take(10))
}

/// Create the first of `names` in `dir` that does not exist yet. O_EXCL
/// never follows a file or symlink someone planted at a name. On Unix the
/// file starts with the mode of `old`, the file it will replace (minus the
/// umask), so it is never more open than that file.
fn create_new_file(dir: &Path, old: Option<&fs::Metadata>, names: impl IntoIterator<Item = String>) -> std::io::Result<(PathBuf, fs::File)> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    if let Some(old) = old {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        opts.mode(old.permissions().mode() & 0o777);
    }
    #[cfg(not(unix))]
    let _ = old;
    let mut taken = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
    for name in names {
        let path = dir.join(name);
        match opts.open(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => taken = e,
            r => return r.map(|f| (path, f)),
        }
    }
    Err(taken)
}

fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

fn stat_from_meta(path: String, md: &fs::Metadata) -> FileStat {
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    FileStat {
        path,
        kind: if md.is_dir() { EntryKind::Dir } else { EntryKind::File },
        size: if md.is_dir() { 0 } else { md.len() },
        mtime,
    }
}

impl VaultFs for StdFs {
    fn list(&self, dir: &str) -> Result<Vec<FileStat>> {
        Ok(self.walk(dir, None)?.0)
    }

    fn list_partial(&self, dir: &str) -> Result<(Vec<FileStat>, Vec<String>)> {
        self.walk(dir, Some(Vec::new()))
    }

    fn under_skipped_link(&self, path: &str) -> bool {
        self.follow(path).looped
    }

    fn real_path(&self, path: &str) -> Option<String> {
        let way = self.follow(path);
        (way.looped && !way.ends_at_loop).then_some(way.real)
    }

    /// Checked again on disk: a name that leads somewhere else now (a link
    /// changed since the listing) is left out. One that is gone, or cannot
    /// be looked up (no permission), still counts until the next listing:
    /// it may have moved (a link renamed) after the listing.
    fn other_names(&self, path: &str) -> Vec<String> {
        let Some(others) = self.names.read().get(path).cloned() else { return Vec::new() };
        match self.id_now(path) {
            Ok(Some(id)) => others
                .into_iter()
                .filter(|o| match self.id_now(o) {
                    Ok(Some(other)) => other == id,
                    Ok(None) | Err(()) => true,
                })
                .collect(),
            Ok(None) => Vec::new(),
            Err(()) => others,
        }
    }

    fn same_file(&self, a: &str, b: &str) -> bool {
        matches!((self.id_now(a), self.id_now(b)), (Ok(Some(x)), Ok(Some(y))) if x == y)
    }

    fn via_link(&self, path: &str) -> bool {
        let abs = self.abs(path);
        let Ok(rel) = abs.strip_prefix(&self.root) else { return false };
        let mut p = self.root.clone();
        rel.iter().any(|c| {
            p.push(c);
            fs::symlink_metadata(&p).is_ok_and(|md| md.file_type().is_symlink())
        })
    }

    fn leads_outside(&self, path: &str) -> bool {
        // The deepest part of the path that exists (a dangling link does),
        // with every link on the way followed: anything missing below it
        // would be made there.
        let abs = self.abs(path);
        let mut p = abs.as_path();
        while fs::symlink_metadata(p).is_err() {
            match p.parent() {
                Some(up) => p = up,
                None => return true,
            }
        }
        let Ok(real) = p.canonicalize() else {
            return true;
        };
        if real == p {
            return false; // no link on the way
        }
        let Ok(rel) = real.strip_prefix(&self.root) else {
            return true;
        };
        // Where the path really is. A link that stays in the vault can still
        // lead a note to .git/config, settings or a data file, which a save
        // through the link would then rewrite.
        let rest = abs.strip_prefix(p).unwrap_or(Path::new(""));
        let real: Vec<_> = rel.join(rest).iter().map(|c| c.to_string_lossy().into_owned()).collect();
        let real = real.join("/");
        vpath::is_hidden(&real) || (vpath::is_markdown(path) && !vpath::is_markdown(&real))
    }

    fn stat(&self, path: &str) -> Result<Option<FileStat>> {
        self.stat_abs(path, &self.op_abs(path)?)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        fs::read(self.op_abs(path)?).map_err(|e| CoreError::io(path, e))
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<FileStat> {
        self.write_following(path, data, true)
    }

    fn write_in_vault(&self, path: &str, data: &[u8]) -> Result<FileStat> {
        self.write_following(path, data, false)
    }

    fn folder_outside(&self, dir: &str) -> Option<String> {
        let names: Vec<&str> = dir.split('/').filter(|c| !c.is_empty()).collect();
        let abs = self.abs(dir);
        let mut p = self.root.clone();
        // From the vault root down, so the folder named is the first one
        // that leads out; whatever is below a missing folder is made in it.
        for (i, c) in abs.strip_prefix(&self.root).ok()?.iter().take(names.len()).enumerate() {
            p.push(c);
            if fs::symlink_metadata(&p).is_err() {
                return None;
            }
            if !p.canonicalize().is_ok_and(|real| real.starts_with(&self.root)) {
                return Some(names[..=i].join("/"));
            }
        }
        None
    }

    fn write_if(&self, path: &str, data: &[u8], check: &dyn Fn(&[u8]) -> bool) -> Result<FileStat> {
        let abs = self.write_target(self.op_abs(path)?, true);
        let conflict = || CoreError::Conflict(path.to_string());
        // Taken before the content is read, so any write after it shows.
        let old = match fs::metadata(&abs) {
            Ok(md) => md,
            Err(e) if is_gone(&e) => return Err(conflict()),
            Err(e) => return Err(CoreError::io(path, e)),
        };
        match fs::read(&abs) {
            Ok(cur) if check(&cur) => {}
            Ok(_) => return Err(conflict()),
            Err(e) if is_gone(&e) => return Err(conflict()),
            Err(e) => return Err(CoreError::io(path, e)),
        }
        self.write_abs(path, &abs, data, Some(&old), true)
    }

    fn create_dir(&self, path: &str) -> Result<()> {
        fs::create_dir_all(self.op_abs(path)?).map_err(|e| CoreError::io(path, e))
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        let a = self.op_abs(from)?;
        let b = self.op_abs(to)?;
        if !a.exists() {
            return Err(CoreError::NotFound(from.to_string()));
        }
        // On a case-insensitive file system `to` can name `from` itself
        // (a.md -> A.md); that rename is allowed. On a case-sensitive one
        // the same names are two different files, and rename(2) would
        // silently replace the other one.
        if fs::symlink_metadata(&b).is_ok() && !same_entry(&a, &b) {
            return Err(CoreError::AlreadyExists(to.to_string()));
        }
        if let Some(p) = b.parent()
            && !p.is_dir()
        {
            return Err(CoreError::NotFound(vpath::parent(to).to_string()));
        }
        fs::rename(&a, &b).map_err(|e| CoreError::io(from, e))?;
        // What the listing saw in folders below `a` now applies below `b`.
        let mut listed = self.listed.write();
        let moved: Vec<PathBuf> = listed.keys().filter(|k| k.starts_with(&a)).cloned().collect();
        for k in moved {
            let names = listed.remove(&k).unwrap_or_default();
            let rest = k.strip_prefix(&a).unwrap_or(Path::new(""));
            listed.insert(if rest.as_os_str().is_empty() { b.clone() } else { b.join(rest) }, names);
        }
        Ok(())
    }

    fn remove(&self, path: &str) -> Result<()> {
        if path.is_empty() {
            return Err(CoreError::InvalidPath("cannot delete the notebook folder".into()));
        }
        let abs = self.op_abs(path)?;
        if fs::symlink_metadata(&abs).is_err() {
            return Err(CoreError::NotFound(path.to_string()));
        }
        match self.trash {
            TrashMode::Permanent => {
                let r = if abs.is_dir() { fs::remove_dir_all(&abs) } else { fs::remove_file(&abs) };
                r.map_err(|e| CoreError::io(path, e))
            }
            TrashMode::Vault => self.move_to_vault_trash(path),
            TrashMode::System => {
                #[cfg(feature = "system-trash")]
                {
                    match trash::delete(&abs) {
                        Ok(()) => return Ok(()),
                        Err(e) => log::warn!("system trash failed for {path}: {e}; using the notebook's .trash folder"),
                    }
                }
                self.move_to_vault_trash(path)
            }
        }
    }

    fn remove_empty_dir(&self, path: &str) -> Result<bool> {
        if path.is_empty() {
            return Ok(false);
        }
        let abs = self.op_abs(path)?;
        // A symlink to a folder is a link the user made, not a folder of
        // the vault: it stays when the folder it leads to is empty, as
        // after sync deleted the last note through it (FINDING-224).
        if fs::symlink_metadata(&abs).is_ok_and(|md| md.file_type().is_symlink()) {
            return Ok(false);
        }
        match fs::read_dir(&abs) {
            Ok(mut rd) => {
                if rd.next().is_some() {
                    return Ok(false);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(CoreError::io(path, e)),
        }
        fs::remove_dir(&abs).map_err(|e| CoreError::io(path, e))?;
        Ok(true)
    }

    fn describe(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn os_path(&self, path: &str) -> Option<PathBuf> {
        Some(self.abs(path))
    }

    fn skipped_backslash_names(&self) -> Vec<String> {
        let listed = self.listed.read();
        let mut names: Vec<String> = listed
            .iter()
            .flat_map(|(dir, l)| l.backslash.iter().map(move |n| dir.join(n)))
            .filter(|p| fs::symlink_metadata(p).is_ok())
            .filter_map(|p| Some(p.strip_prefix(&self.root).ok()?.to_str()?.to_string()))
            .collect();
        names.sort();
        names
    }

    /// On Unix, the inode number and the status change time, which the
    /// system sets on every write, rename or mtime change and which no tool
    /// can set back.
    #[cfg(unix)]
    fn change_stamp(&self, path: &str) -> Option<u64> {
        use std::os::unix::fs::MetadataExt;
        let md = fs::metadata(self.op_abs(path).ok()?).ok()?;
        let mut b = Vec::with_capacity(24);
        for n in [md.ino(), md.ctime() as u64, md.ctime_nsec() as u64] {
            b.extend_from_slice(&n.to_le_bytes());
        }
        let h = crate::index::hash_bytes(&b);
        Some(u64::from_le_bytes(h[..8].try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, StdFs) {
        let d = tempfile::tempdir().unwrap();
        let fs = StdFs::new(d.path(), TrashMode::Vault).unwrap();
        (d, fs)
    }

    #[test]
    fn write_read_list() {
        let (_d, fs) = setup();
        fs.create_dir("a/b").unwrap();
        fs.write("a/b/n.md", b"hello").unwrap();
        fs.write("top.md", b"x").unwrap();
        assert_eq!(fs.read("a/b/n.md").unwrap(), b"hello");
        let mut paths: Vec<_> = fs.list("").unwrap().into_iter().map(|s| s.path).collect();
        paths.sort();
        assert_eq!(paths, vec!["a", "a/b", "a/b/n.md", "top.md"]);
        let sub: Vec<_> = fs.list("a").unwrap().into_iter().map(|s| s.path).collect();
        assert_eq!(sub.len(), 2);
        // No temp files left behind
        let names: Vec<_> = std::fs::read_dir(fs.root().join("a/b")).unwrap().collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn write_needs_parent() {
        let (_d, fs) = setup();
        assert!(matches!(fs.write("missing/n.md", b""), Err(CoreError::NotFound(_))));
    }

    #[cfg(unix)]
    #[test]
    fn read_only_files_are_not_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, fs) = setup();
        fs.write("ro.md", b"keep").unwrap();
        let abs = fs.root().join("ro.md");
        std::fs::set_permissions(&abs, std::fs::Permissions::from_mode(0o444)).unwrap();
        let w = fs.write("ro.md", b"new");
        let wi = fs.write_if("ro.md", b"new", &|_| true);
        std::fs::set_permissions(&abs, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(&w, Err(CoreError::Io(m)) if m.contains("read-only")), "{w:?}");
        assert!(matches!(&wi, Err(CoreError::Io(m)) if m.contains("read-only")), "{wi:?}");
        assert_eq!(std::fs::read(&abs).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(fs.root()).unwrap().count(), 1, "temp file left behind");
    }

    #[test]
    fn write_if_needs_the_file_with_the_checked_content() {
        let (_d, fs) = setup();
        fs.write("n.md", b"old").unwrap();
        assert!(matches!(fs.write_if("n.md", b"new", &|cur| cur == b"other"), Err(CoreError::Conflict(_))));
        assert!(matches!(fs.write_if("gone.md", b"new", &|_| true), Err(CoreError::Conflict(_))));
        assert!(matches!(fs.write_if("missing/n.md", b"new", &|_| true), Err(CoreError::Conflict(_))));
        assert!(!fs.root().join("gone.md").exists());
        assert_eq!(fs.read("n.md").unwrap(), b"old");
        fs.write_if("n.md", b"new", &|cur| cur == b"old").unwrap();
        assert_eq!(fs.read("n.md").unwrap(), b"new");
        assert_eq!(std::fs::read_dir(fs.root()).unwrap().count(), 1, "temp file left behind");
    }

    #[cfg(unix)]
    #[test]
    fn write_if_loses_to_a_save_after_the_check() {
        let (_d, fs) = setup();
        let abs = fs.root().join("n.md");
        let other = fs.root().join("other");
        let in_place = || std::fs::write(&abs, "theirs, longer").unwrap();
        let same_size_old_mtime = || {
            let t = std::fs::metadata(&abs).unwrap().modified().unwrap();
            std::fs::write(&abs, "THEIRS").unwrap();
            std::fs::File::options().write(true).open(&abs).unwrap().set_modified(t).unwrap();
        };
        let replaced = || {
            std::fs::write(&other, "theirs").unwrap();
            std::fs::rename(&other, &abs).unwrap();
        };
        let saves: [(&str, &dyn Fn()); 3] = [("in place", &in_place), ("same size, old mtime", &same_size_old_mtime), ("replaced", &replaced)];
        for (name, save) in saves {
            std::fs::write(&abs, "mine 1").unwrap();
            // Older kernels keep the change time within one clock tick (a
            // few ms); start their save in a later one.
            std::thread::sleep(std::time::Duration::from_millis(20));
            // `check` runs once the content is read, so what it writes lands
            // between the check and the rename, like another program's save.
            let r = fs.write_if("n.md", b"mine 2", &|cur| {
                save();
                cur == b"mine 1"
            });
            assert!(matches!(r, Err(CoreError::Conflict(_))), "{name}: {r:?}");
            assert_ne!(std::fs::read(&abs).unwrap(), b"mine 2", "{name}: their save was overwritten");
            assert_eq!(std::fs::read_dir(fs.root()).unwrap().count(), 1, "{name}: temp file left behind");
        }
    }

    #[cfg(unix)]
    #[test]
    fn writes_keep_symlinks_and_hard_links() {
        use std::os::unix::fs::{symlink, MetadataExt};
        let (_d, fs) = setup();
        let r = fs.root();
        let is_link = |p: &str| std::fs::symlink_metadata(r.join(p)).unwrap().file_type().is_symlink();
        std::fs::create_dir(r.join("real")).unwrap();
        std::fs::write(r.join("real/n.md"), "old").unwrap();
        symlink("real/n.md", r.join("rel.md")).unwrap();
        symlink("rel.md", r.join("chain.md")).unwrap();
        symlink(r.join("nowhere.md"), r.join("dangling.md")).unwrap();
        fs.write("chain.md", b"one").unwrap();
        fs.write_if("rel.md", b"two", &|cur| cur == b"one").unwrap();
        assert_eq!(std::fs::read(r.join("real/n.md")).unwrap(), b"two");
        assert!(is_link("rel.md") && is_link("chain.md"));
        // A dangling link leads to no file: the note replaces the link.
        fs.write("dangling.md", b"new").unwrap();
        assert!(!is_link("dangling.md") && !r.join("nowhere.md").exists());
        // A hard-linked file keeps its other names, through a link too.
        std::fs::hard_link(r.join("real/n.md"), r.join("hard.md")).unwrap();
        fs.write_if("hard.md", b"three, longer", &|cur| cur == b"two").unwrap();
        fs.write("rel.md", b"four").unwrap();
        assert_eq!(std::fs::read(r.join("hard.md")).unwrap(), b"four");
        assert_eq!(std::fs::metadata(r.join("hard.md")).unwrap().nlink(), 2);
        assert!(is_link("rel.md"));
        let left = |d: &Path| std::fs::read_dir(d).unwrap().count();
        assert_eq!((left(r), left(&r.join("real"))), (5, 1), "temp file left behind");
    }

    #[cfg(unix)]
    #[test]
    fn temp_files_never_follow_or_reuse_a_planted_name() {
        let (_d, fs) = setup();
        let o = tempfile::tempdir().unwrap();
        std::fs::write(o.path().join("victim"), "victim").unwrap();
        std::os::unix::fs::symlink(o.path().join("victim"), fs.root().join(".a")).unwrap();
        std::os::unix::fs::symlink(o.path().join("new"), fs.root().join(".b")).unwrap();
        let names = [".a", ".b", ".c"].map(String::from);
        let (tmp, mut f) = create_new_file(fs.root(), None, names.clone()).unwrap();
        f.write_all(b"mine").unwrap();
        assert_eq!(tmp, fs.root().join(".c"));
        assert_eq!(std::fs::read_to_string(o.path().join("victim")).unwrap(), "victim");
        assert!(!o.path().join("new").exists());
        let e = create_new_file(fs.root(), None, names).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn hidden_entries_are_skipped() {
        let (_d, fs) = setup();
        fs.create_dir(".cairn").unwrap();
        fs.write(".cairn/x.json", b"{}").unwrap();
        fs.write("n.md", b"").unwrap();
        let paths: Vec<_> = fs.list("").unwrap().into_iter().map(|s| s.path).collect();
        assert_eq!(paths, vec!["n.md"]);
    }

    #[test]
    fn rename_refuses_overwrite_but_allows_case_change() {
        let (_d, fs) = setup();
        fs.write("a.md", b"a").unwrap();
        fs.write("b.md", b"b").unwrap();
        assert!(matches!(fs.rename("a.md", "b.md"), Err(CoreError::AlreadyExists(_))));
        fs.rename("a.md", "A.md").unwrap();
        assert_eq!(fs.read("A.md").unwrap(), b"a");
    }

    #[cfg(unix)]
    #[test]
    fn case_only_rename_onto_another_entry_is_refused() {
        let (_d, fs) = setup();
        fs.write("a.md", b"lower").unwrap();
        fs.write("A.md", b"upper").unwrap();
        assert!(matches!(fs.rename("a.md", "A.md"), Err(CoreError::AlreadyExists(_))));
        // A symlink to the file is still a different entry: moving the link
        // over its own target would leave a link that points at itself.
        fs.write("B.md", b"bee").unwrap();
        std::os::unix::fs::symlink("B.md", fs.root().join("b.md")).unwrap();
        assert!(matches!(fs.rename("b.md", "B.md"), Err(CoreError::AlreadyExists(_))));
        assert_eq!(fs.read("a.md").unwrap(), b"lower");
        assert_eq!(fs.read("A.md").unwrap(), b"upper");
        assert_eq!(fs.read("B.md").unwrap(), b"bee");
    }

    // macOS file systems treat names in different Unicode forms as one
    // name, so neither the on-disk spellings nor twins below can exist.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn names_in_another_unicode_form_are_used_in_place() {
        let (_d, fs) = setup();
        // Made outside Cairn, not listed yet: the NFD form is found.
        std::fs::create_dir(fs.root().join("Re\u{301}sume\u{301}")).unwrap();
        std::fs::write(fs.root().join("Re\u{301}sume\u{301}/cafe\u{301}.md"), b"mac").unwrap();
        assert_eq!(fs.read("R\u{e9}sum\u{e9}/caf\u{e9}.md").unwrap(), b"mac");
        fs.write("R\u{e9}sum\u{e9}/caf\u{e9}.md", b"edited").unwrap();
        fs.write("R\u{e9}sum\u{e9}/new\u{e9}.md", b"new").unwrap();
        let dir = fs.root().join("Re\u{301}sume\u{301}");
        let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, vec!["cafe\u{301}.md", "new\u{e9}.md"]);
        assert_eq!(std::fs::read(dir.join("cafe\u{301}.md")).unwrap(), b"edited");
        // Forms only a listing can find (Kelvin sign).
        std::fs::write(fs.root().join("\u{212a}elvin.md"), b"k").unwrap();
        let mut paths: Vec<_> = fs.list("").unwrap().into_iter().map(|s| s.path).collect();
        paths.sort();
        assert_eq!(paths, vec!["Kelvin.md", "R\u{e9}sum\u{e9}", "R\u{e9}sum\u{e9}/caf\u{e9}.md", "R\u{e9}sum\u{e9}/new\u{e9}.md"]);
        assert_eq!(fs.read("Kelvin.md").unwrap(), b"k");
        fs.rename("Kelvin.md", "K.md").unwrap();
        assert_eq!(std::fs::read(fs.root().join("K.md")).unwrap(), b"k");
        assert_eq!(
            fs.to_vault_path(&fs.root().join("Re\u{301}sume\u{301}/cafe\u{301}.md")).as_deref(),
            Some("R\u{e9}sum\u{e9}/caf\u{e9}.md")
        );
    }

    /// Names that differ only in Unicode form are never merged: one has the
    /// vault name, each other one a twin name that reaches it.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn unicode_twins_are_listed_under_twin_names() {
        let (_d, fs) = setup();
        let r = fs.root();
        std::fs::write(r.join("caf\u{e9}.md"), b"nfc").unwrap();
        std::fs::write(r.join("cafe\u{301}.md"), b"nfd").unwrap();
        // Without an NFC name, the NFD one has the vault name, before and
        // after a listing (Angstrom sign, A + ring, A with ring). A twin name
        // that is taken, also in another case, is skipped.
        std::fs::write(r.join("\u{212b}.md"), b"sign").unwrap();
        std::fs::write(r.join("A\u{30a}.md"), b"nfd A").unwrap();
        std::fs::write(r.join("\u{c5} (unicode TWIN).md"), b"taken").unwrap();
        assert_eq!(fs.read("\u{c5}.md").unwrap(), b"nfd A");
        let mut listed: Vec<_> = fs.list("").unwrap().into_iter().map(|s| (s.path, s.size)).collect();
        listed.sort();
        let want = [
            ("caf\u{e9} (Unicode twin).md", 3),
            ("caf\u{e9}.md", 3),
            ("\u{c5} (Unicode twin 2).md", 4),
            ("\u{c5} (unicode TWIN).md", 5),
            ("\u{c5}.md", 5),
        ];
        assert_eq!(listed, want.map(|(p, n)| (p.to_string(), n)));
        assert_eq!(fs.read("\u{c5}.md").unwrap(), b"nfd A");
        // Each is read, written and renamed in place.
        fs.write("caf\u{e9}.md", b"nfc edited").unwrap();
        fs.write("caf\u{e9} (Unicode twin).md", b"nfd edited").unwrap();
        assert_eq!(std::fs::read(r.join("caf\u{e9}.md")).unwrap(), b"nfc edited");
        assert_eq!(std::fs::read(r.join("cafe\u{301}.md")).unwrap(), b"nfd edited");
        assert_eq!(fs.read("\u{c5} (Unicode twin 2).md").unwrap(), b"sign");
        fs.rename("\u{c5} (Unicode twin 2).md", "sign.md").unwrap();
        assert_eq!(std::fs::read(r.join("sign.md")).unwrap(), b"sign");
        assert_eq!(fs.read("\u{c5}.md").unwrap(), b"nfd A");
        let mut names: Vec<_> = std::fs::read_dir(r).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["A\u{30a}.md", "cafe\u{301}.md", "caf\u{e9}.md", "sign.md", "\u{c5} (unicode TWIN).md"]);
        // Until the next listing, a twin is only found by its twin name.
        fs.remove("caf\u{e9}.md").unwrap();
        assert!(fs.stat("caf\u{e9}.md").unwrap().is_none());
        assert_eq!(fs.read("caf\u{e9} (Unicode twin).md").unwrap(), b"nfd edited");
        assert!(paths(fs.list("").unwrap()).contains(&"caf\u{e9}.md".to_string()));
        assert_eq!(fs.read("caf\u{e9}.md").unwrap(), b"nfd edited");
    }

    /// The watcher's mapping gives the folder of a name that is, or may be,
    /// listed under a twin name, so that the folder is rescanned.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn watcher_paths_of_twins_are_their_folder() {
        let (_d, fs) = setup();
        let d = fs.root().join("d");
        std::fs::create_dir(&d).unwrap();
        for n in ["caf\u{e9}.md", "cafe\u{301}.md", "A\u{30a}.md", "\u{212b}.md", "\u{212a}elvin.md"] {
            std::fs::write(d.join(n), n).unwrap();
        }
        // Before (a separate mapper never lists) and after a listing.
        let mapper = StdFs::new(fs.root(), TrashMode::Vault).unwrap();
        fs.list("").unwrap();
        for m in [&mapper, &fs] {
            let map = |n: &str| m.to_vault_path(&d.join(n));
            assert_eq!(map("caf\u{e9}.md").as_deref(), Some("d/caf\u{e9}.md"));
            assert_eq!(map("cafe\u{301}.md").as_deref(), Some("d"));
            assert_eq!(map("A\u{30a}.md").as_deref(), Some("d/\u{c5}.md"));
            assert_eq!(map("\u{212b}.md").as_deref(), Some("d"));
            assert_eq!(map("\u{212a}elvin.md").as_deref(), Some("d/Kelvin.md"));
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_forms_follow_a_folder_rename() {
        let (_d, fs) = setup();
        std::fs::create_dir_all(fs.root().join("A/sub")).unwrap();
        std::fs::write(fs.root().join("A/sub/\u{212a}elvin.md"), b"k").unwrap();
        fs.list("").unwrap();
        fs.rename("A", "B").unwrap();
        assert_eq!(fs.read("B/sub/Kelvin.md").unwrap(), b"k");
        fs.write("B/sub/Kelvin.md", b"edited").unwrap();
        let names: Vec<_> = std::fs::read_dir(fs.root().join("B/sub")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec!["\u{212a}elvin.md"]);
        assert_eq!(fs.read("B/sub/Kelvin.md").unwrap(), b"edited");
    }

    /// Looking up a name in another form must not read its folder: doing
    /// that for every file made opening a large Mac-made folder quadratic.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn lookups_never_read_the_whole_folder() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, fs) = setup();
        let dir = fs.root().join("d");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("cafe\u{301}.md"), b"nfd").unwrap();
        std::fs::write(dir.join("\u{212a}elvin.md"), b"k").unwrap();
        fs.list("").unwrap();
        // Names in a folder without read permission can still be looked up,
        // but the folder cannot be listed.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
        if std::fs::read_dir(&dir).is_ok() {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            return; // running as root
        }
        let nfd = fs.read("d/caf\u{e9}.md");
        let kelvin = fs.read("d/Kelvin.md");
        let edited = fs.write("d/caf\u{e9}.md", b"edited").map(|st| st.size);
        let created = fs.write("d/new\u{e9}.md", b"new").map(|st| st.path);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(nfd.unwrap(), b"nfd");
        assert_eq!(kelvin.unwrap(), b"k");
        assert_eq!(edited.unwrap(), 6);
        assert_eq!(created.unwrap(), "d/new\u{e9}.md");
        let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, vec!["cafe\u{301}.md", "new\u{e9}.md", "\u{212a}elvin.md"]);
    }

    /// A backslash is an ordinary character in Linux names but a separator
    /// in vault paths: such names are skipped, and reported as skipped.
    #[cfg(unix)]
    #[test]
    fn names_with_a_backslash_are_skipped() {
        let (_d, fs) = setup();
        let r = fs.root();
        std::fs::write(r.join("back\\slash.md"), b"b").unwrap();
        std::fs::write(r.join(OsStr::new("\\")), b"x").unwrap(); // a file named '\'
        std::fs::create_dir_all(r.join("sub/a\\b")).unwrap();
        std::fs::write(r.join("sub/a\\b/inside.md"), b"i").unwrap();
        std::fs::create_dir(r.join(".cairn")).unwrap();
        std::fs::write(r.join(".cairn/x\\y.js"), b"js").unwrap();
        fs.write("ok.md", b"ok").unwrap();
        assert!(fs.skipped_backslash_names().is_empty());
        assert_eq!(paths(fs.list("").unwrap()), vec!["ok.md", "sub"]);
        assert_eq!(paths(fs.list(".cairn").unwrap()), Vec::<String>::new());
        assert_eq!(fs.skipped_backslash_names(), ["\\", "back\\slash.md", "sub/a\\b"]);
        assert_eq!(fs.to_vault_path(&r.join("back\\slash.md")), None);
        assert_eq!(fs.to_vault_path(r).as_deref(), Some(""));
        // The list follows a folder moved in Cairn, and leaves out a name
        // that is gone.
        fs.rename("sub", "moved").unwrap();
        std::fs::rename(r.join("back\\slash.md"), r.join("back slash.md")).unwrap();
        assert_eq!(fs.skipped_backslash_names(), ["\\", "moved/a\\b"]);
        assert_eq!(paths(fs.list("").unwrap()), vec!["back slash.md", "moved", "ok.md"]);
        std::fs::remove_dir_all(r.join("moved/a\\b")).unwrap();
        assert_eq!(fs.skipped_backslash_names(), ["\\"]);
    }

    #[test]
    fn remove_to_vault_trash() {
        let (_d, fs) = setup();
        fs.write("a.md", b"1").unwrap();
        fs.remove("a.md").unwrap();
        fs.write("a.md", b"2").unwrap();
        fs.remove("a.md").unwrap();
        assert!(fs.stat("a.md").unwrap().is_none());
        assert_eq!(std::fs::read(fs.root().join(".trash/a.md")).unwrap(), b"1");
        assert_eq!(std::fs::read(fs.root().join(".trash/a 1.md")).unwrap(), b"2");
        assert!(fs.remove("").is_err());
    }

    fn paths(list: Vec<FileStat>) -> Vec<String> {
        let mut p: Vec<String> = list.into_iter().map(|s| s.path).collect();
        p.sort();
        p
    }

    /// Only a folder that leads back to one the listing is in is skipped.
    /// Every other link is followed, even when it shows a folder twice.
    #[cfg(unix)]
    #[test]
    fn only_links_that_loop_back_are_skipped() {
        use std::os::unix::fs::symlink;
        let (d, fs) = setup();
        let outside = tempfile::tempdir().unwrap();
        let (r, o) = (d.path(), outside.path());
        fs.write("n.md", b"").unwrap();
        fs.create_dir("a").unwrap();
        fs.write("a/x.md", b"").unwrap();
        fs.create_dir(".hidden").unwrap();
        fs.write(".hidden/h.md", b"").unwrap();
        std::fs::create_dir(o.join("ext")).unwrap();
        std::fs::write(o.join("ext/e.md"), "").unwrap();
        symlink(".", r.join("loop")).unwrap(); // the vault itself
        symlink("..", r.join("a/up")).unwrap(); // a parent
        symlink("a", r.join("alias")).unwrap(); // a folder of the vault: shown twice
        symlink(".hidden", r.join("shown")).unwrap();
        symlink(o.join("ext"), r.join("ext")).unwrap(); // a folder outside
        symlink(o, r.join("zz")).unwrap(); // ... and the folder around it
        symlink(".", o.join("ext/self")).unwrap(); // a loop outside the vault
        symlink(r, o.join("ext/vault")).unwrap(); // back to the vault
        let all = ["a", "a/x.md", "alias", "alias/x.md", "ext", "ext/e.md", "n.md", "shown", "shown/h.md", "zz", "zz/ext", "zz/ext/e.md"];
        assert_eq!(paths(fs.list("").unwrap()), all);
        assert_eq!(paths(fs.list("zz/ext").unwrap()), vec!["zz/ext/e.md"]);
        assert_eq!(paths(fs.list("alias").unwrap()), vec!["alias/x.md"]);
        for p in ["loop", "a/up", "ext/self", "zz/ext/vault"] {
            assert!(matches!(fs.list(p), Err(CoreError::InvalidPath(_))), "{p}");
        }
        for p in ["loop", "loop/n.md", "loop/a", "a/up", "a/up/n.md", "alias/up", "ext/self/e.md", "ext/vault", "zz/ext/self"] {
            assert!(fs.under_skipped_link(p), "{p}");
        }
        for p in ["", "a", "a/x.md", "n.md", "alias/x.md", "shown", "ext/e.md", "zz/ext/e.md", "missing/x.md"] {
            assert!(!fs.under_skipped_link(p), "{p}");
        }
        let real = [
            ("loop", None),
            ("a/up", None),
            ("loop/loop", None),
            ("n.md", None),
            ("loop/n.md", Some("n.md")),
            ("loop/a/x.md", Some("a/x.md")),
            ("a/up/a/up/n.md", Some("n.md")),
            ("alias/up/a/x.md", Some("a/x.md")),
            ("ext/self/e.md", Some("ext/e.md")),
            ("ext/vault/a", Some("a")),
            ("loop/new/deeper.md", Some("new/deeper.md")),
        ];
        for (p, real) in real {
            assert_eq!(fs.real_path(p).as_deref(), real, "{p}");
        }
    }

    /// Nothing is read, written, moved or deleted through a loop: every
    /// operation on a path through one fails.
    #[cfg(unix)]
    #[test]
    fn operations_through_a_loop_fail() {
        use std::os::unix::fs::symlink;
        let (d, fs) = setup();
        let r = d.path();
        fs.write("n.md", b"note").unwrap();
        fs.create_dir("a").unwrap();
        fs.write("a/x.md", b"x").unwrap();
        symlink(".", r.join("loop")).unwrap();
        symlink("..", r.join("a/up")).unwrap();
        let refused = |res: Result<()>| matches!(res, Err(CoreError::InvalidPath(m)) if m.contains("loops back"));
        for p in ["loop", "loop/n.md", "a/up/n.md", "loop/a/x.md", "loop/a", "loop/new.md"] {
            assert!(refused(fs.stat(p).map(drop)), "stat {p}");
            assert!(refused(fs.read(p).map(drop)), "read {p}");
            assert!(refused(fs.write(p, b"over").map(drop)), "write {p}");
            assert!(refused(fs.write_if(p, b"over", &|_| true).map(drop)), "write_if {p}");
            assert!(refused(fs.create_dir(&format!("{p}/sub"))), "create_dir {p}");
            assert!(refused(fs.rename(p, "moved")), "rename {p}");
            assert!(refused(fs.rename("n.md", &format!("{p}/moved.md"))), "rename to {p}");
            assert!(refused(fs.remove(p)), "remove {p}");
            assert!(refused(fs.remove_empty_dir(p).map(drop)), "remove_empty_dir {p}");
            assert!(refused(fs.list(p).map(drop)), "list {p}");
        }
        assert_eq!(std::fs::read(r.join("n.md")).unwrap(), b"note");
        assert_eq!(std::fs::read(r.join("a/x.md")).unwrap(), b"x");
        let mut names: Vec<_> = std::fs::read_dir(r).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, vec!["a", "loop", "n.md"]);
        // The vault's own paths still work.
        fs.write("a/x.md", b"edited").unwrap();
        fs.rename("a/x.md", "a/y.md").unwrap();
        assert_eq!(fs.read("a/y.md").unwrap(), b"edited");
    }

    /// A listing of the whole vault finds the files it reaches under more
    /// than one vault path: through a folder linked in twice, or a link to
    /// a note. A hard link is a file of its own, and a loop is not listed.
    #[cfg(unix)]
    #[test]
    fn other_names_of_a_file_reached_twice() {
        use std::os::unix::fs::symlink;
        let (d, fs) = setup();
        let outside = tempfile::tempdir().unwrap();
        let (r, o) = (d.path(), outside.path());
        fs.create_dir("notes").unwrap();
        fs.write("notes/x.md", b"x").unwrap();
        fs.write("n.md", b"n").unwrap();
        fs.write("solo.md", b"s").unwrap();
        std::fs::create_dir(o.join("shared")).unwrap();
        std::fs::write(o.join("shared/s.md"), "s").unwrap();
        symlink("notes", r.join("alias")).unwrap();
        symlink(o.join("shared"), r.join("projects")).unwrap();
        symlink(o.join("shared"), r.join("work")).unwrap();
        symlink("n.md", r.join("link.md")).unwrap();
        symlink("../n.md", r.join("notes/up.md")).unwrap();
        std::fs::hard_link(r.join("n.md"), r.join("hard.md")).unwrap();
        symlink(".", r.join("loop")).unwrap();
        // Only a listing of the whole vault finds them.
        fs.list("notes").unwrap();
        assert!(fs.other_names("notes/x.md").is_empty());
        fs.list("").unwrap();
        assert_eq!(fs.other_names("notes/x.md"), ["alias/x.md"]);
        assert_eq!(fs.other_names("alias/x.md"), ["notes/x.md"]);
        assert_eq!(fs.other_names("work/s.md"), ["projects/s.md"]);
        assert_eq!(fs.other_names("n.md"), ["alias/up.md", "link.md", "notes/up.md"]);
        assert_eq!(fs.other_names("alias/up.md"), ["link.md", "n.md", "notes/up.md"]);
        for p in ["solo.md", "hard.md", "notes", "alias", "projects", "missing.md", "loop/n.md"] {
            assert!(fs.other_names(p).is_empty(), "{p}");
        }
        for (p, linked) in [("notes/x.md", false), ("n.md", false), ("hard.md", false), ("alias/x.md", true), ("link.md", true), ("notes/up.md", true), ("work/s.md", true)] {
            assert_eq!(fs.via_link(p), linked, "{p}");
        }
        assert!(fs.same_file("alias/x.md", "notes/x.md") && fs.same_file("link.md", "n.md"));
        assert!(!fs.same_file("hard.md", "n.md") && !fs.same_file("solo.md", "n.md") && !fs.same_file("alias/new.md", "notes/new.md"));
        // A listing of one folder keeps them; the next one of the whole
        // vault follows the disk.
        fs.list_partial("alias").unwrap();
        assert_eq!(fs.other_names("notes/x.md"), ["alias/x.md"]);
        std::fs::remove_file(r.join("alias")).unwrap();
        fs.list_partial("").unwrap();
        assert!(fs.other_names("notes/x.md").is_empty());
        assert_eq!(fs.other_names("n.md"), ["link.md", "notes/up.md"]);
        // A name that leads somewhere else now is no other name any more.
        // One that is gone may have moved: it counts until the next listing.
        std::fs::remove_file(r.join("link.md")).unwrap();
        symlink("solo.md", r.join("link.md")).unwrap();
        assert_eq!(fs.other_names("n.md"), ["notes/up.md"]);
        std::fs::rename(r.join("work"), r.join("job")).unwrap();
        assert_eq!(fs.other_names("projects/s.md"), ["work/s.md"]);
        fs.list_partial("").unwrap();
        assert_eq!(fs.other_names("projects/s.md"), ["job/s.md"]);
    }

    /// What a partial listing cannot read is unknown, not gone: the other
    /// names found there before still count.
    #[cfg(unix)]
    #[test]
    fn other_names_in_a_folder_that_cannot_be_read_still_count() {
        use std::os::unix::fs::PermissionsExt;
        let (d, fs) = setup();
        let r = d.path();
        fs.create_dir("notes").unwrap();
        fs.write("notes/x.md", b"x").unwrap();
        std::fs::create_dir(r.join("private")).unwrap();
        std::os::unix::fs::symlink("../notes", r.join("private/link")).unwrap();
        fs.list_partial("").unwrap();
        assert_eq!(fs.other_names("notes/x.md"), ["private/link/x.md"]);
        std::fs::set_permissions(r.join("private"), std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(r.join("private")).is_ok() {
            std::fs::set_permissions(r.join("private"), std::fs::Permissions::from_mode(0o755)).unwrap();
            return; // running as root
        }
        let partial = fs.list_partial("");
        let (x, link) = (fs.other_names("notes/x.md"), fs.other_names("private/link/x.md"));
        std::fs::set_permissions(r.join("private"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(partial.unwrap().1, ["private"]);
        assert_eq!((x, link), (vec!["private/link/x.md".to_string()], vec!["notes/x.md".to_string()]));
        fs.list_partial("").unwrap();
        assert_eq!(fs.other_names("notes/x.md"), ["private/link/x.md"]);
    }

    /// A folder link stays when the folder it leads to is emptied: it is the
    /// user's link, not an empty folder of the vault (FINDING-224).
    #[cfg(unix)]
    #[test]
    fn an_emptied_folder_link_is_not_removed() {
        use std::os::unix::fs::symlink;
        let (d, fs) = setup();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(outside.path().join("empty")).unwrap();
        symlink(outside.path().join("empty"), d.path().join("out")).unwrap();
        fs.create_dir("real/sub").unwrap();
        symlink("real/sub", d.path().join("in")).unwrap();
        assert!(!fs.remove_empty_dir("out").unwrap());
        assert!(!fs.remove_empty_dir("in").unwrap());
        for link in ["out", "in"] {
            assert!(std::fs::symlink_metadata(d.path().join(link)).unwrap().file_type().is_symlink(), "{link}");
        }
        assert!(outside.path().join("empty").is_dir());
        assert!(fs.remove_empty_dir("real/sub").unwrap());
        assert!(!d.path().join("real/sub").exists());
    }

    /// An entry that cannot be looked up in a folder that can (a link into
    /// a folder this user may not enter) is unknown, not gone.
    #[cfg(unix)]
    #[test]
    fn an_entry_that_cannot_be_looked_up_is_unreadable() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (d, fs) = setup();
        let outside = tempfile::tempdir().unwrap();
        let locked = outside.path().join("locked");
        std::fs::create_dir_all(locked.join("sub")).unwrap();
        std::fs::write(locked.join("o.md"), "").unwrap();
        fs.write("n.md", b"").unwrap();
        fs.create_dir("refs").unwrap();
        symlink(locked.join("o.md"), d.path().join("refs/o.md")).unwrap();
        symlink(locked.join("sub"), d.path().join("refs/sub")).unwrap();
        symlink(locked.join("o.md"), d.path().join("top.md")).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&locked).is_ok() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            return; // running as root
        }
        let (strict, partial) = (fs.list(""), fs.list_partial(""));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(strict.is_err());
        let (list, unreadable) = partial.unwrap();
        assert_eq!(paths(list), vec!["n.md", "refs"]);
        assert_eq!(unreadable, vec!["refs/o.md", "refs/sub", "top.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_fails_a_listing_but_not_a_partial_one() {
        use std::os::unix::fs::PermissionsExt;
        let (d, fs) = setup();
        fs.create_dir("locked/sub").unwrap();
        fs.write("locked/x.md", b"").unwrap();
        fs.write("n.md", b"").unwrap();
        let locked = d.path().join("locked");
        // No permission at all, or names that can be read but not opened.
        for mode in [0o000, 0o644] {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(mode)).unwrap();
            let (strict, partial, one) = (fs.list(""), fs.list_partial(""), fs.list_partial("locked"));
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(strict.is_err(), "mode {mode:o}");
            let (list, unreadable) = partial.unwrap();
            assert_eq!((paths(list), unreadable), (vec!["locked".to_string(), "n.md".to_string()], vec!["locked".to_string()]));
            assert_eq!(one.unwrap(), (vec![], vec!["locked".to_string()]), "mode {mode:o}");
        }
    }

    /// A folder of symlinks only, with names readable but entries not
    /// reachable (mode 644), is unreadable too, not empty.
    #[cfg(unix)]
    #[test]
    fn a_folder_of_links_without_search_permission_is_unreadable() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (d, fs) = setup();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("o.md"), "").unwrap();
        fs.create_dir("refs").unwrap();
        let refs = d.path().join("refs");
        symlink(outside.path().join("o.md"), refs.join("o.md")).unwrap();
        symlink(outside.path(), refs.join("dir")).unwrap();
        std::fs::set_permissions(&refs, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (strict, partial) = (fs.list(""), fs.list_partial(""));
        std::fs::set_permissions(&refs, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(strict.is_err());
        let (list, unreadable) = partial.unwrap();
        assert_eq!((paths(list), unreadable), (vec!["refs".to_string()], vec!["refs".to_string()]));
        // A link whose target is gone is still skipped on its own.
        std::fs::remove_file(outside.path().join("o.md")).unwrap();
        assert_eq!(paths(fs.list("").unwrap()), vec!["refs", "refs/dir"]);
    }

    #[test]
    fn a_folder_replaced_by_a_file_is_not_an_error() {
        let (_d, fs) = setup();
        fs.write("f", b"").unwrap();
        assert_eq!(fs.stat("f/a.md").unwrap(), None);
        assert!(fs.list("f").unwrap().is_empty());
    }

    #[test]
    fn a_vault_folder_that_is_gone_fails_the_listing() {
        let (_d, fs) = setup();
        std::fs::remove_dir(fs.root()).unwrap();
        assert!(fs.list("").is_err() && fs.list_partial("").is_err());
        std::fs::write(fs.root(), "").unwrap();
        assert!(fs.list("").is_err() && fs.list_partial("").is_err());
        std::fs::remove_file(fs.root()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn change_stamp_sees_writes_that_keep_size_and_mtime() {
        let (_d, fs) = setup();
        fs.write("a.md", b"teh cat").unwrap();
        fs.write("b.md", b"the dog").unwrap();
        let p = fs.root().join("a.md");
        let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
        let s1 = fs.change_stamp("a.md");
        assert!(s1.is_some());
        fs.read("a.md").unwrap();
        fs.stat("a.md").unwrap();
        assert_eq!(fs.change_stamp("a.md"), s1, "looking at a file does not change it");
        // Past the file system's timestamp step (kernels without fine
        // grained change times share one per tick).
        std::thread::sleep(std::time::Duration::from_millis(20));
        // A same-size edit by a tool that puts the mtime back.
        std::fs::write(&p, "the cat").unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(mtime).unwrap();
        let s2 = fs.change_stamp("a.md");
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), mtime);
        assert_ne!(s2, s1);
        // Another file of that size and mtime moved over it.
        std::fs::File::options().write(true).open(fs.root().join("b.md")).unwrap().set_modified(mtime).unwrap();
        std::fs::rename(fs.root().join("b.md"), &p).unwrap();
        assert!(![s1, s2].contains(&fs.change_stamp("a.md")));
        assert_eq!(fs.change_stamp("b.md"), None);
    }
}
